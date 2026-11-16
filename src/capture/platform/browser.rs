//! Source « Navigateur » (Windows) : une page web (alertes, widgets) affichée
//! par WebView2 dans une fenêtre hors de l'écran, capturée avec sa
//! transparence par Windows Graphics Capture, et son son capturé à part.
//!
//! La fenêtre n'a pas de surface propre (WS_EX_NOREDIRECTIONBITMAP) : les
//! zones transparentes de la page le restent dans la capture. Chaque page a
//! son propre dossier WebView2, donc ses propres processus, ce qui permet de
//! capturer son son seul.

use super::*;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use std::num::NonZeroIsize;
use std::sync::Once;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, TranslateMessage, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Interface, w};
use windows_capture::window::Window;
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

const CLASS: windows::core::PCWSTR = w!("MyStreamNavigateur");

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// La fenêtre hôte, pour wry.
struct Host(isize);

impl HasWindowHandle for Host {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(NonZeroIsize::new(self.0).ok_or(HandleError::Unavailable)?));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// Adresse à ouvrir : fichier local, adresse sans « https:// », ou page vide.
fn normalize(url: &str) -> String {
    let u = url.trim();
    if u.is_empty() {
        return "about:blank".into();
    }
    let p = std::path::Path::new(u);
    if p.is_absolute() && p.exists() {
        return format!("file:///{}", u.replace('\\', "/").replace(' ', "%20"));
    }
    if u.contains("://") || u.starts_with("about:") || u.starts_with("data:") { u.to_string() } else { format!("https://{u}") }
}

/// Dossier WebView2 propre à la page (cookies gardés d'une fois sur l'autre).
fn data_dir(url: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    dirs::config_dir().unwrap_or_else(|| ".".into()).join("MyStream-natif").join("navigateur").join(format!("{:016x}", h.finish()))
}

fn css_script(css: &str) -> String {
    let css = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(() => {{ const add = () => {{ const s = document.createElement('style'); s.textContent = {css}; \
         (document.head || document.documentElement).appendChild(s); }}; \
         if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', add); else add(); }})();"
    )
}

#[allow(clippy::too_many_arguments)]
pub fn start_browser(url: String, width: u32, height: u32, css: String, fps: u32, slot: Slot, status: Arc<Status>, track: Option<Arc<Track>>) {
    std::thread::spawn(move || {
        if let Err(e) = run(&url, width, height, &css, fps, &slot, &status, track) {
            status.fail(format!("Navigateur : {e}"));
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn run(url: &str, width: u32, height: u32, css: &str, fps: u32, slot: &Slot, status: &Arc<Status>, track: Option<Arc<Track>>) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() };
            RegisterClassW(&class);
        });
        // Hors de tous les écrans, à gauche de l'écran le plus à gauche.
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN) - width as i32 - 200;
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!("MyStream - source navigateur"),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            width as i32,
            height as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| format!("fenêtre impossible à créer : {e}"))?;
        let raw = hwnd.0 as isize;

        let mut context = WebContext::new(Some(data_dir(url)));
        // Pas de ralentissement de la page quand sa fenêtre n'est pas visible.
        let args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion \
                    --disable-background-timer-throttling --disable-renderer-backgrounding \
                    --disable-backgrounding-occluded-windows --autoplay-policy=no-user-gesture-required";
        let built = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(normalize(url))
            .with_transparent(true)
            .with_initialization_script(css_script(css))
            .with_additional_browser_args(args)
            .with_default_context_menus(false)
            .with_devtools(false)
            .build(&Host(raw));
        let webview = match built {
            Ok(w) => w,
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                return Err(format!("WebView2 indisponible ({e}). Installez « Microsoft Edge WebView2 Runtime »."));
            }
        };
        // 1 pixel CSS = 1 pixel de la source, quelle que soit la mise à l'échelle de Windows.
        if let Ok(c3) = webview.controller().cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller3>() {
            let _ = c3.SetShouldDetectMonitorScaleChanges(false);
            let _ = c3.SetRasterizationScale(1.0);
        }
        let mut pid = 0u32;
        let _ = webview.webview().BrowserProcessId(&mut pid);
        let audio = track.filter(|_| pid != 0).map(|t| process_audio(pid, status.clone(), t));

        // Capture dans un autre thread ; celui-ci fait vivre la page.
        let capture = {
            let (slot, status) = (slot.clone(), status.clone());
            std::thread::spawn(move || {
                while !status.stopped() {
                    let window = Window::from_raw_hwnd(raw as *mut std::ffi::c_void);
                    if let Err(e) = capture_window(window, fps, &slot, &status, true, || true) {
                        *status.error.lock() = Some(format!("Capture de la page impossible : {e}"));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
        };

        let mut msg = MSG::default();
        while !status.stopped() {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        if let Some(a) = audio {
            a.store(true, Ordering::Relaxed);
        }
        let _ = capture.join();
        drop(webview);
        let _ = DestroyWindow(hwnd);
        Ok(())
    }
}
