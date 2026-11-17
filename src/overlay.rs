//! Source « Chat » : le chat du live dessiné dans l'image du stream, comme
//! l'incrustation de la version Electron : logo de la plateforme, badges
//! (images officielles ou logos dessinés), pseudo en couleur, emotes, contour
//! du texte, messages les plus récents en bas, disparition après un délai.

use crate::capture::{Frame, Slot, Status};
use crate::chat::{ChatHandle, Message, Part, badge_svg, platform_name, platform_svg};
use crate::model::{CHAT_FONTS, ChatStyle};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

pub struct Fonts {
    pub regular: fontdue::Font,
    pub bold: fontdue::Font,
}

/// Police choisie (fichiers de Windows), sinon celle intégrée.
pub fn fonts(name: &str) -> Arc<Fonts> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Fonts>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(f) = cache.lock().get(name) {
        return f.clone();
    }
    let (_, regular, bold) = CHAT_FONTS.iter().find(|(n, _, _)| *n == name).copied().unwrap_or(CHAT_FONTS[1]);
    let dir = std::env::var("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let load = |files: &[&str], fallback: &'static [u8]| {
        let bytes = files.iter().find_map(|n| std::fs::read(dir.join(n)).ok()).unwrap_or_else(|| fallback.to_vec());
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .unwrap_or_else(|_| fontdue::Font::from_bytes(fallback, fontdue::FontSettings::default()).expect("police"))
    };
    let f = Arc::new(Fonts {
        regular: load(&[regular, "segoeui.ttf"], epaint_default_fonts::UBUNTU_LIGHT),
        bold: load(&[bold, "seguisb.ttf", "segoeuib.ttf"], epaint_default_fonts::UBUNTU_LIGHT),
    });
    cache.lock().insert(name.to_string(), f.clone());
    f
}

/// Images des emotes et badges, téléchargées une fois (None : en cours ou échec).
type Images = Arc<Mutex<HashMap<String, Option<Arc<image::RgbaImage>>>>>;

fn web_image(images: &Images, url: &str) -> Option<Arc<image::RgbaImage>> {
    let mut map = images.lock();
    if let Some(v) = map.get(url) {
        return v.clone();
    }
    map.insert(url.to_string(), None);
    let (images, url) = (images.clone(), url.to_string());
    std::thread::spawn(move || {
        let img = ureq::get(&url)
            .call()
            .ok()
            .and_then(|mut r| r.body_mut().with_config().limit(4 << 20).read_to_vec().ok())
            .and_then(|b| image::load_from_memory(&b).ok())
            .map(|i| Arc::new(i.to_rgba8()));
        images.lock().insert(url, img);
    });
    None
}

/// Logo SVG (plateforme, badge dessiné) rendu à la taille voulue.
fn svg_image(svg: &str, px: u32) -> Option<Arc<image::RgbaImage>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u32), Option<Arc<image::RgbaImage>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (svg.to_string(), px);
    if let Some(v) = cache.lock().get(&key) {
        return v.clone();
    }
    let img = (|| {
        let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(px.max(1), px.max(1))?;
        let k = px as f32 / tree.size().width().max(1.0);
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
        let data: Vec<u8> = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        image::RgbaImage::from_raw(px, px, data).map(Arc::new)
    })();
    cache.lock().insert(key, img.clone());
    img
}

struct Canvas {
    w: i32,
    h: i32,
    data: Vec<u8>,
}

impl Canvas {
    /// Mélange un pixel (alpha non prémultiplié).
    fn blend(&mut self, x: i32, y: i32, rgb: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let da = self.data[i + 3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        for k in 0..3 {
            let v = (rgb[k] as f32 * a + self.data[i + k] as f32 * da * (1.0 - a)) / oa.max(1e-6);
            self.data[i + k] = v.round() as u8;
        }
        self.data[i + 3] = (oa * 255.0).round() as u8;
    }

    /// Dessine un caractère ; avec `outline`, d'abord un contour noir autour.
    fn glyph(&mut self, font: &fontdue::Font, c: char, x: f32, baseline: f32, size: f32, rgb: [u8; 3], alpha: f32, outline: f32) -> f32 {
        let (m, bmp) = font.rasterize(c, size);
        let gx = x.round() as i32 + m.xmin;
        let gy = baseline.round() as i32 - m.height as i32 - m.ymin;
        if outline > 0.0 {
            let r = outline.round().max(1.0) as i32;
            for row in 0..m.height {
                for col in 0..m.width {
                    let a = bmp[row * m.width + col] as f32 / 255.0;
                    if a <= 0.05 {
                        continue;
                    }
                    for dy in -r..=r {
                        for dx in -r..=r {
                            if dx * dx + dy * dy <= r * r {
                                self.blend(gx + col as i32 + dx, gy + row as i32 + dy, [0, 0, 0], a * 0.85 * alpha * 0.35);
                            }
                        }
                    }
                }
            }
        }
        for row in 0..m.height {
            for col in 0..m.width {
                self.blend(gx + col as i32, gy + row as i32, rgb, bmp[row * m.width + col] as f32 / 255.0 * alpha);
            }
        }
        m.advance_width
    }

    /// Image redimensionnée (bilinéaire) en (x, y), largeur w et hauteur h.
    fn image(&mut self, img: &image::RgbaImage, x: f32, y: f32, w: f32, h: f32, alpha: f32) {
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        for py in 0..h.ceil() as i32 {
            for px in 0..w.ceil() as i32 {
                let sx = ((px as f32 + 0.5) / w * iw - 0.5).clamp(0.0, iw - 1.0);
                let sy = ((py as f32 + 0.5) / h * ih - 0.5).clamp(0.0, ih - 1.0);
                let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
                let (x1, y1) = ((x0 + 1).min(img.width() - 1), (y0 + 1).min(img.height() - 1));
                let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                let mut acc = [0f32; 4];
                for (xx, yy, wgt) in [(x0, y0, (1.0 - fx) * (1.0 - fy)), (x1, y0, fx * (1.0 - fy)), (x0, y1, (1.0 - fx) * fy), (x1, y1, fx * fy)] {
                    let p = img.get_pixel(xx, yy).0;
                    let a = p[3] as f32 * wgt;
                    for k in 0..3 {
                        acc[k] += p[k] as f32 * a;
                    }
                    acc[3] += a;
                }
                if acc[3] > 0.0 {
                    let rgb = [(acc[0] / acc[3]) as u8, (acc[1] / acc[3]) as u8, (acc[2] / acc[3]) as u8];
                    self.blend(x as i32 + px, y as i32 + py, rgb, acc[3] / 255.0 * alpha);
                }
            }
        }
    }

    fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: [u8; 3], alpha: f32) {
        for py in 0..h.ceil() as i32 {
            for px in 0..w.ceil() as i32 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let dx = (r - fx).max(fx - (w - r)).max(0.0);
                let dy = (r - fy).max(fy - (h - r)).max(0.0);
                let cover = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                self.blend(x as i32 + px, y as i32 + py, rgb, cover * alpha);
            }
        }
    }
}

/// Élément d'une ligne.
enum Token {
    Text { text: String, bold: bool, rgb: [u8; 3] },
    Image { img: Arc<image::RgbaImage>, w: f32, h: f32 },
    Label(String),
}

fn measure(f: &fontdue::Font, text: &str, size: f32) -> f32 {
    text.chars().map(|c| f.metrics(c, size).advance_width).sum()
}

fn rgb3(c: [u8; 4]) -> [u8; 3] {
    [c[0], c[1], c[2]]
}

fn tokens(m: &Message, st: &ChatStyle, images: &Images, size: f32) -> Vec<Token> {
    let mut out = Vec::new();
    let icon_px = size.round() as u32;
    if m.system {
        out.push(Token::Text { text: format!("[{}]", platform_name(&m.platform)), bold: true, rgb: [0x9a, 0xa0, 0xb4] });
    } else {
        if st.show_icons
            && let Some(img) = platform_svg(&m.platform).and_then(|s| svg_image(&s, icon_px * 2))
        {
            out.push(Token::Image { img, w: size, h: size });
        }
        if st.show_badges {
            for b in &m.badges {
                if let Some(img) = b.image.as_deref().and_then(|u| web_image(images, u)) {
                    out.push(Token::Image { w: size * img.width() as f32 / img.height().max(1) as f32, img, h: size });
                } else if let Some(img) = badge_svg(&b.kind).and_then(|s| svg_image(&s, icon_px * 2)) {
                    out.push(Token::Image { img, w: size, h: size });
                } else {
                    out.push(Token::Label(b.label()));
                }
            }
        }
        out.push(Token::Text { text: m.username.clone(), bold: true, rgb: m.color.unwrap_or(rgb3(st.name_color)) });
        out.push(Token::Text { text: ":".into(), bold: false, rgb: rgb3(st.text_color) });
    }
    for p in &m.parts {
        match p {
            Part::Text(t) => {
                for w in t.split_whitespace() {
                    out.push(Token::Text { text: w.to_string(), bold: false, rgb: rgb3(st.text_color) });
                }
            }
            Part::Emote { name, url } => match web_image(images, url) {
                Some(img) => {
                    let h = size * 1.3;
                    out.push(Token::Image { w: h * img.width() as f32 / img.height().max(1) as f32, img, h });
                }
                None => out.push(Token::Text { text: name.clone(), bold: false, rgb: rgb3(st.text_color) }),
            },
        }
    }
    out
}

/// Dessine les messages (les plus récents en bas) ; renvoie aussi le prochain instant où redessiner (fondu).
pub fn render(messages: &[Message], st: &ChatStyle, images: &Images, now: Instant) -> (Frame, Option<Instant>) {
    let f = fonts(&st.font);
    let (w, h) = (st.width.max(50) as i32, st.height.max(50) as i32);
    let mut c = Canvas { w, h, data: st.background.iter().copied().cycle().take((w * h * 4) as usize).collect() };
    let size = st.size.max(8.0);
    let line_h = (size * 1.4).round();
    let pad = (size * 0.5).round();
    let gap = size * 0.3;
    let max_w = w as f32 - pad * 2.0;
    let outline = if st.shadow { (size * 0.08).max(1.0) } else { 0.0 };
    let life = Duration::from_secs(st.lifetime as u64);
    let mut next: Option<Instant> = None;
    let mut y = h as f32 - pad;
    for m in messages.iter().rev().filter(|m| !m.divider && (!m.system || st.show_system)).take(st.max_messages.max(1) as usize) {
        // Disparition : fondu pendant la dernière seconde.
        let mut alpha = 1.0f32;
        if st.lifetime > 0 {
            let Some(at) = m.at else { continue };
            let age = now.duration_since(at);
            if age > life {
                continue;
            }
            alpha = ((life - age).as_secs_f32()).min(1.0);
            let fade = at + life.saturating_sub(Duration::from_secs(1));
            let when = if fade > now { fade } else { now + Duration::from_millis(50) };
            next = Some(next.map_or(when, |n| n.min(when)));
        }
        // Mise en page en lignes.
        let mut rows: Vec<Vec<(f32, Token, f32)>> = vec![Vec::new()];
        let mut x = 0.0f32;
        for t in tokens(m, st, images, size) {
            let tw = match &t {
                Token::Text { text, bold, .. } => measure(if *bold { &f.bold } else { &f.regular }, text, size),
                Token::Image { w, .. } => *w,
                Token::Label(l) => measure(&f.bold, l, size * 0.6) + size * 0.5,
            };
            let colon = matches!(&t, Token::Text { text, .. } if text == ":");
            let space = if x > 0.0 && !colon { gap } else { 0.0 };
            if x > 0.0 && x + space + tw > max_w {
                rows.push(Vec::new());
                x = 0.0;
            } else {
                x += space;
            }
            rows.last_mut().unwrap().push((x, t, tw));
            x += tw;
        }
        let height = rows.len() as f32 * line_h;
        if y - height < 0.0 {
            break; // pas de message coupé en haut
        }
        y -= height;
        for (r, row) in rows.into_iter().enumerate() {
            let cy = y + r as f32 * line_h + line_h / 2.0;
            for (x, t, tw) in row {
                let x = pad + x;
                match t {
                    Token::Text { text, bold, rgb } => {
                        let font = if bold { &f.bold } else { &f.regular };
                        let baseline = cy + size * 0.35;
                        let mut cx = x;
                        for ch in text.chars() {
                            cx += c.glyph(font, ch, cx, baseline, size, rgb, alpha, outline);
                        }
                    }
                    Token::Image { img, w, h } => c.image(&img, x, cy - h / 2.0, w, h, alpha),
                    Token::Label(l) => {
                        let bh = size * 0.85;
                        c.round_rect(x, cy - bh / 2.0, tw, bh, bh / 4.0, [0x4c, 0x7d, 0xff], 0.85 * alpha);
                        let mut cx = x + size * 0.25;
                        for ch in l.chars() {
                            cx += c.glyph(&f.bold, ch, cx, cy + size * 0.22, size * 0.6, [255, 255, 255], alpha, 0.0);
                        }
                    }
                }
            }
        }
        y -= gap;
    }
    (Frame::new(w as u32, h as u32, c.data), next)
}

/// Redessine la source quand le chat, les images ou ses réglages changent.
pub fn start(chat: ChatHandle, style: ChatStyle, slot: Slot, status: Arc<Status>) {
    std::thread::spawn(move || {
        let images: Images = Arc::default();
        let mut last = (u64::MAX, 0usize);
        let mut next: Option<Instant> = None;
        while !status.stop.load(Ordering::Relaxed) {
            let v = chat.version.load(Ordering::Relaxed);
            let loaded = images.lock().values().filter(|i| i.is_some()).count();
            let now = Instant::now();
            if (v, loaded) != last || next.is_some_and(|n| now >= n) {
                last = (v, loaded);
                let (frame, n) = render(&chat.messages(), &style, &images, now);
                next = n;
                slot.set(frame);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}
