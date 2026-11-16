//! Source « Chat » : le chat du live dessiné dans l'image du stream (pseudos
//! en couleur, badges, images des emotes), les messages les plus récents en
//! bas, comme l'incrustation de la version Electron.

use crate::capture::{Frame, Slot, Status};
use crate::chat::{ChatHandle, Message, Part};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

struct Fonts {
    regular: fontdue::Font,
    bold: fontdue::Font,
}

fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let load = |names: &[&str], fallback: &'static [u8]| {
            let dir = std::env::var("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts")).unwrap_or_default();
            let bytes = names.iter().find_map(|n| std::fs::read(dir.join(n)).ok()).unwrap_or_else(|| fallback.to_vec());
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).expect("police")
        };
        Fonts {
            regular: load(&["segoeui.ttf"], epaint_default_fonts::UBUNTU_LIGHT),
            bold: load(&["seguisb.ttf", "segoeuib.ttf"], epaint_default_fonts::UBUNTU_LIGHT),
        }
    })
}

/// Images des emotes, téléchargées une fois (None : en cours ou échec).
type Images = Arc<Mutex<HashMap<String, Option<Arc<image::RgbaImage>>>>>;

fn emote(images: &Images, url: &str) -> Option<Arc<image::RgbaImage>> {
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

    fn glyph(&mut self, font: &fontdue::Font, c: char, x: f32, baseline: f32, size: f32, rgb: [u8; 3]) -> f32 {
        let (m, bmp) = font.rasterize(c, size);
        let gx = x.round() as i32 + m.xmin;
        let gy = baseline.round() as i32 - m.height as i32 - m.ymin;
        for row in 0..m.height {
            for col in 0..m.width {
                self.blend(gx + col as i32, gy + row as i32, rgb, bmp[row * m.width + col] as f32 / 255.0);
            }
        }
        m.advance_width
    }

    fn image(&mut self, img: &image::RgbaImage, x: f32, y: f32, h: f32) -> f32 {
        let w = img.width() as f32 * h / img.height().max(1) as f32;
        for py in 0..h.ceil() as i32 {
            for px in 0..w.ceil() as i32 {
                let sx = ((px as f32 / w) * img.width() as f32) as u32;
                let sy = ((py as f32 / h) * img.height() as f32) as u32;
                let p = img.get_pixel(sx.min(img.width() - 1), sy.min(img.height() - 1)).0;
                self.blend(x as i32 + px, y as i32 + py, [p[0], p[1], p[2]], p[3] as f32 / 255.0);
            }
        }
        w
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3]) {
        for py in y as i32..(y + h) as i32 {
            for px in x as i32..(x + w) as i32 {
                self.blend(px, py, rgb, 1.0);
            }
        }
    }
}

/// Élément d'une ligne : texte (avec police et couleur), emote ou badge.
enum Token {
    Text { text: String, bold: bool, rgb: [u8; 3] },
    Emote(String, String),
    Badge([u8; 3], char),
}

fn measure(f: &fontdue::Font, text: &str, size: f32) -> f32 {
    text.chars().map(|c| f.metrics(c, size).advance_width).sum()
}

fn tokens(m: &Message) -> Vec<Token> {
    let mut out = Vec::new();
    for b in &m.badges {
        if let Some((label, rgb)) = crate::chat::badge(b) {
            out.push(Token::Badge(rgb, label.chars().next().unwrap_or('•')));
        }
    }
    let name_rgb = m.color.unwrap_or(crate::chat::platform_color(&m.platform));
    out.push(Token::Text { text: format!("{} ", m.username), bold: true, rgb: name_rgb });
    for p in &m.parts {
        match p {
            Part::Text(t) => {
                // Mot par mot, pour pouvoir passer à la ligne.
                for w in t.split_inclusive(' ') {
                    out.push(Token::Text { text: w.to_string(), bold: false, rgb: [0xf2, 0xf3, 0xf7] });
                }
            }
            Part::Emote { name, url } => out.push(Token::Emote(name.clone(), url.clone())),
        }
    }
    out
}

/// Dessine les messages (les plus récents en bas) dans une image RGBA.
pub fn render(messages: &[Message], width: u32, height: u32, size: f32, background: [u8; 4], max: usize, images: &Images) -> Frame {
    let f = fonts();
    let (w, h) = (width.max(16) as i32, height.max(16) as i32);
    let mut c = Canvas { w, h, data: background.iter().copied().cycle().take((w * h * 4) as usize).collect() };
    let line_h = size * 1.35;
    let pad = size * 0.5;
    let max_w = w as f32 - pad * 2.0;
    // Mise en page de chaque message en lignes (positions x, numéro de ligne).
    let mut blocks: Vec<Vec<(f32, usize, Token)>> = Vec::new();
    for m in messages.iter().rev().take(max.max(1)) {
        let mut placed = Vec::new();
        let (mut x, mut line) = (0.0f32, 0usize);
        for t in tokens(m) {
            let tw = match &t {
                Token::Text { text, bold, .. } => measure(if *bold { &f.bold } else { &f.regular }, text, size),
                Token::Emote(name, url) => emote(images, url)
                    .map(|i| i.width() as f32 * size * 1.2 / i.height().max(1) as f32 + size * 0.2)
                    .unwrap_or_else(|| measure(&f.regular, name, size) + size * 0.3),
                Token::Badge(..) => size * 1.25,
            };
            if x + tw > max_w && x > 0.0 {
                x = 0.0;
                line += 1;
            }
            placed.push((x, line, t));
            x += tw;
        }
        blocks.push(placed);
    }
    // Du bas vers le haut.
    let mut bottom = h as f32 - pad;
    for block in blocks {
        let lines = block.last().map(|b| b.1 + 1).unwrap_or(1);
        let top = bottom - lines as f32 * line_h;
        if top < 0.0 {
            break;
        }
        for (x, line, t) in block {
            let y = top + line as f32 * line_h;
            let baseline = y + size * 1.05;
            let x = pad + x;
            match t {
                Token::Text { text, bold, rgb } => {
                    let font = if bold { &f.bold } else { &f.regular };
                    let mut cx = x;
                    for ch in text.chars() {
                        cx += c.glyph(font, ch, cx, baseline, size, rgb);
                    }
                }
                Token::Emote(name, url) => match emote(images, &url) {
                    Some(img) => {
                        c.image(&img, x, y + size * 0.05, size * 1.2);
                    }
                    None => {
                        let mut cx = x;
                        for ch in name.chars() {
                            cx += c.glyph(&f.regular, ch, cx, baseline, size, [0xf2, 0xf3, 0xf7]);
                        }
                    }
                },
                Token::Badge(rgb, letter) => {
                    let s = size * 1.05;
                    c.rect(x, y + size * 0.15, s, s, rgb);
                    c.glyph(&f.bold, letter, x + s * 0.28, y + size * 0.15 + s * 0.78, s * 0.7, [255, 255, 255]);
                }
            }
        }
        bottom = top - size * 0.35;
    }
    Frame::new(w as u32, h as u32, c.data)
}

/// Redessine la source quand le chat ou ses réglages changent.
pub fn start(chat: ChatHandle, width: u32, height: u32, size: f32, background: [u8; 4], max: u32, slot: Slot, status: Arc<Status>) {
    std::thread::spawn(move || {
        let images: Images = Arc::default();
        let mut last = (u64::MAX, 0usize);
        while !status.stop.load(Ordering::Relaxed) {
            let v = chat.version.load(Ordering::Relaxed);
            let loaded = images.lock().values().filter(|i| i.is_some()).count();
            if (v, loaded) != last {
                last = (v, loaded);
                slot.set(render(&chat.messages(), width, height, size, background, max as usize, &images));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}
