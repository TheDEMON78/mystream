//! Chat : flux agrégé de Restream (Twitch, Kick…) ou chat Twitch lu en
//! direct (anonyme, sans compte), avec badges, emotes et modération
//! (messages supprimés, viewers bannis), comme la version Electron.

use crate::model::{ChatSettings, RestreamSettings};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_MESSAGES: usize = 200;

#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    Emote { name: String, url: String },
}

/// Badge d'un viewer (modérateur, abonné…), avec son image officielle si la plateforme la donne.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Badge {
    pub kind: String,
    pub title: String,
    pub image: Option<String>,
    pub count: Option<u64>,
}

impl Badge {
    fn of(kind: &str) -> Self {
        Self { kind: kind.replace('-', "_"), ..Default::default() }
    }
    /// Nom affiché au survol.
    pub fn label(&self) -> String {
        let l = badge_label(&self.kind).map(String::from).unwrap_or_else(|| if self.title.is_empty() { self.kind.clone() } else { self.title.clone() });
        match self.count {
            Some(n) => format!("{l} ({n})"),
            None => l,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: String,
    pub user_id: String,
    pub platform: String,
    pub username: String,
    /// Couleur du pseudo (#rrggbb), si la plateforme en donne une.
    pub color: Option<[u8; 3]>,
    pub parts: Vec<Part>,
    pub badges: Vec<Badge>,
    /// Message système (connexion, information).
    pub system: bool,
    /// Séparateur « reprise du direct » après l'historique rejoué par Restream.
    pub divider: bool,
    /// Le message cite le streamer.
    pub mentioned: bool,
    /// Arrivée (None : historique rejoué), pour masquer les vieux messages dans le stream.
    pub at: Option<Instant>,
}

impl Message {
    #[allow(dead_code)]
    pub fn plain_text(&self) -> String {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => t.clone(),
                Part::Emote { name, .. } => name.clone(),
            })
            .collect()
    }
}

pub fn platform_name(p: &str) -> &str {
    match p {
        "twitch" => "Twitch",
        "kick" => "Kick",
        "youtube" => "YouTube",
        "restream" => "Restream",
        other => other,
    }
}

pub fn platform_color(p: &str) -> [u8; 3] {
    match p {
        "twitch" => [0xa9, 0x70, 0xff],
        "kick" => [0x53, 0xfc, 0x18],
        "youtube" => [0xff, 0x33, 0x33],
        _ => [0x4c, 0x7d, 0xff],
    }
}

/// Logos officiels des plateformes (simple-icons, CC0), les mêmes que Restream.
pub fn platform_svg(p: &str) -> Option<String> {
    let path = match p {
        "twitch" => "M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z",
        "kick" => "M1.333 0h8v5.333H12V2.667h2.667V0h8v8H20v2.667h-2.667v2.666H20V16h2.667v8h-8v-2.667H12v-2.666H9.333V24h-8Z",
        _ => return None,
    };
    let [r, g, b] = platform_color(p);
    Some(format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="48" height="48"><path fill="#{r:02x}{g:02x}{b:02x}" d="{path}"/></svg>"##))
}

/// Libellé d'un badge (le même que la version Electron).
pub fn badge_label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "broadcaster" => "Diffuseur",
        "moderator" => "Modo",
        "vip" => "VIP",
        "subscriber" => "Abonné",
        "founder" => "Fondateur",
        "staff" => "Staff",
        "admin" => "Admin",
        "partner" => "Partenaire",
        "turbo" => "Turbo",
        "premium" => "Prime",
        "bits" => "Bits",
        "sub_gifter" => "Cadeau abo",
        "og" => "OG",
        "verified" => "Vérifié",
        _ => return None,
    })
}

/// Logo dessiné d'un badge sans image (Kick n'en fournit pas toujours) :
/// fond coloré et pictogramme blanc, comme la version Electron.
pub fn badge_svg(kind: &str) -> Option<String> {
    let (bg, d) = match kind {
        "broadcaster" => ("#e91916", "M4 7h10v10H4zM14 10.5l5-3v9l-5-3z"),
        "moderator" => ("#00ad03", "M15 4h5v5L10 19l-5-5zM4 20l3-3"),
        "vip" => ("#e005b9", "M7 5h10l4 5-9 10L3 10zM3 10h18"),
        "subscriber" => ("#8205b4", "M12 4l2.4 5 5.6.6-4.2 3.8 1.2 5.6-5-2.9-5 2.9 1.2-5.6L4 9.6 9.6 9z"),
        "founder" => ("#3d89d8", "M4 17h16M5 17l-1-9 5 4 3-6 3 6 5-4-1 9"),
        "og" => ("#0e9e7a", "M8.5 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM20 9h-3a2 2 0 0 0-2 2v2a2 2 0 0 0 2 2h3v-3h-2"),
        "verified" => ("#1fd655", "M5 12.5l4.5 4.5L19 7.5"),
        "sub_gifter" => ("#ff8c00", "M4 10h16v3H4zM6 13h12v7H6zM12 10v10M12 10c-2-5-6-4-4-.5M12 10c2-5 6-4 4-.5"),
        "staff" => ("#3a3a3a", "M14 6a4 4 0 0 0 5 5l-8 8-3-3 8-8a4 4 0 0 1-2-2z"),
        "partner" => ("#9146ff", "M5 12.5l4.5 4.5L19 7.5"),
        "bits" => ("#8a3dff", "M12 3l7 9-7 9-7-9z"),
        "premium" => ("#00a3ff", "M4 8l4 4 4-7 4 7 4-4-2 10H6z"),
        _ => return None,
    };
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="48" height="48"><rect width="24" height="24" rx="5" fill="{bg}"/><path d="{d}" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/></svg>"##
    ))
}

fn hex(c: &str) -> Option<[u8; 3]> {
    let c = c.trim().trim_start_matches('#');
    if c.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(c, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// Découpe le texte en morceaux de texte et d'emotes (positions Twitch, ou [emote:ID:nom] de Kick).
fn parse_parts(text: &str, emotes: &[(String, usize, usize)]) -> Vec<Part> {
    let mut parts = Vec::new();
    if !emotes.is_empty() {
        let chars: Vec<char> = text.chars().collect();
        let mut sorted = emotes.to_vec();
        sorted.sort_by_key(|e| e.1);
        let mut pos = 0;
        for (id, start, end) in sorted {
            if start >= chars.len() || end >= chars.len() || start < pos {
                continue;
            }
            if start > pos {
                parts.push(Part::Text(chars[pos..start].iter().collect()));
            }
            parts.push(Part::Emote {
                name: chars[start..=end].iter().collect(),
                url: format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0"),
            });
            pos = end + 1;
        }
        if pos < chars.len() {
            parts.push(Part::Text(chars[pos..].iter().collect()));
        }
        return parts;
    }
    let mut rest = text;
    while let Some(i) = rest.find("[emote:") {
        let after = &rest[i + 7..];
        let Some(j) = after.find(']') else { break };
        let inner = &after[..j];
        let Some((id, name)) = inner.split_once(':') else { break };
        if i > 0 {
            parts.push(Part::Text(rest[..i].to_string()));
        }
        parts.push(Part::Emote { name: name.to_string(), url: format!("https://files.kick.com/emotes/{id}/fullsize") });
        rest = &after[j + 1..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_string()));
    }
    parts
}

#[derive(Default)]
struct State {
    messages: VecDeque<Message>,
    status: BTreeMap<String, (String, Option<String>)>,
    last_ts: f64,
    replayed: bool,
    /// Messages reçus depuis le démarrage.
    total: u64,
    /// Images des badges Twitch (« type/version » -> adresse, titre), en lecture directe.
    twitch_badges: HashMap<String, (String, String)>,
    /// Identifiants des messages Kick déjà reçus (la page en reçoit parfois deux fois).
    seen: HashSet<String>,
}

/// Chat en cours (fils de lecture).
pub struct Chat {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    /// Augmente à chaque changement (pour redessiner l'incrustation du chat).
    pub version: Arc<AtomicU64>,
}

impl Default for Chat {
    fn default() -> Self {
        Self { state: Arc::default(), stop: Arc::new(AtomicBool::new(true)), version: Arc::default() }
    }
}

/// « ma_chaine », « kick.com/ma_chaine » -> « ma-chaine » (format des adresses Kick).
pub fn kick_slug(channel: &str) -> String {
    let c = channel.trim().to_lowercase();
    let c = c.rsplit("kick.com/").next().unwrap_or("");
    let c = c.split(['/', '?', '#']).next().unwrap_or("");
    c.replace('_', "-").chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-').collect()
}

fn twitch_login(channel: &str) -> String {
    let c = channel.trim().to_lowercase();
    c.rsplit('/').next().unwrap_or("").chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_').collect()
}

impl Chat {
    /// (Re)démarre la lecture du chat selon les paramètres ; les sources « Chat » gardent le même accès.
    pub fn restart(&mut self, cfg: &ChatSettings, restream: &RestreamSettings) {
        self.stop.store(true, Ordering::Relaxed);
        self.stop = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.state.lock();
            st.status.clear();
            st.last_ts = 0.0;
            st.replayed = false;
        }
        self.version.fetch_add(1, Ordering::Relaxed);
        match cfg.mode.as_str() {
            "restream" => {
                let (state, stop, version, rs) = (self.state.clone(), self.stop.clone(), self.version.clone(), restream.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        crate::restream::chat_stream(
                            &rs,
                            &stop,
                            |s, e| {
                                state.lock().status.insert("restream".into(), (s.into(), e));
                                version.fetch_add(1, Ordering::Relaxed);
                            },
                            |v| {
                                push_restream(&state, v);
                                version.fetch_add(1, Ordering::Relaxed);
                            },
                        );
                        std::thread::sleep(Duration::from_secs(3));
                    }
                });
            }
            // Lecture directe, sans compte : Twitch (IRC anonyme) et Kick (page de chat officielle).
            "direct" | "twitch" => {
                let channel = twitch_login(&cfg.twitch_channel);
                if !channel.is_empty() {
                    let (state, stop, version) = (self.state.clone(), self.stop.clone(), self.version.clone());
                    let login = channel.clone();
                    std::thread::spawn(move || {
                        let map = twitch_badge_map(&login);
                        let mut st = state.lock();
                        st.twitch_badges = map;
                        let State { messages, twitch_badges, .. } = &mut *st;
                        for m in messages.iter_mut() {
                            resolve_twitch_badges(m, twitch_badges);
                        }
                        drop(st);
                        version.fetch_add(1, Ordering::Relaxed);
                    });
                    let (state, stop2, version) = (self.state.clone(), stop.clone(), self.version.clone());
                    std::thread::spawn(move || {
                        while !stop2.load(Ordering::Relaxed) {
                            twitch_irc(&channel, &state, &stop2, &version);
                            if !stop2.load(Ordering::Relaxed) {
                                state.lock().status.insert("twitch".into(), ("error".into(), Some("déconnecté, nouvelle tentative…".into())));
                                std::thread::sleep(Duration::from_secs(3));
                            }
                        }
                    });
                }
                let slug = kick_slug(&cfg.kick_channel);
                if !slug.is_empty() {
                    kick_page(&slug, self.state.clone(), self.stop.clone(), self.version.clone());
                }
            }
            _ => {}
        }
    }

    pub fn messages(&self) -> Vec<Message> {
        self.state.lock().messages.iter().cloned().collect()
    }

    pub fn status(&self) -> Vec<(String, String, Option<String>)> {
        self.state.lock().status.iter().map(|(k, (s, e))| (k.clone(), s.clone(), e.clone())).collect()
    }

    /// Nombre de messages reçus depuis le démarrage.
    pub fn total(&self) -> u64 {
        self.state.lock().total
    }

    pub fn clear(&self) {
        self.state.lock().messages.clear();
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    /// Accès partagé pour l'incrustation du chat dans le stream.
    pub fn handle(&self) -> ChatHandle {
        ChatHandle { state: self.state.clone(), version: self.version.clone() }
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Lecture seule du chat, pour les sources « Chat ».
#[derive(Clone)]
pub struct ChatHandle {
    state: Arc<Mutex<State>>,
    pub version: Arc<AtomicU64>,
}

impl ChatHandle {
    pub fn messages(&self) -> Vec<Message> {
        self.state.lock().messages.iter().cloned().collect()
    }
}

fn add(state: &Mutex<State>, mut m: Message) {
    let mut s = state.lock();
    if m.platform == "twitch" {
        resolve_twitch_badges(&mut m, &s.twitch_badges);
    }
    if !m.system && !m.divider {
        s.total += 1;
    }
    s.messages.push_back(m);
    while s.messages.len() > MAX_MESSAGES {
        s.messages.pop_front();
    }
}

fn message(platform: &str) -> Message {
    Message {
        id: String::new(),
        user_id: String::new(),
        platform: platform.into(),
        username: String::new(),
        color: None,
        parts: Vec::new(),
        badges: Vec::new(),
        system: false,
        divider: false,
        mentioned: false,
        at: Some(Instant::now()),
    }
}

/// Retire ce que la plateforme a supprimé : un message, ceux d'un viewer, ou tout.
fn moderate(state: &Mutex<State>, platform: &str, action: &str, id: &str, user_id: &str, username: &str) {
    let name = username.to_lowercase();
    state.lock().messages.retain(|m| {
        if m.platform != platform || m.system || m.divider {
            return true;
        }
        match action {
            "clear" => false,
            "delete" => id.is_empty() || m.id != id,
            "user" => !((!user_id.is_empty() && m.user_id == user_id) || (!name.is_empty() && m.username.to_lowercase() == name)),
            _ => true,
        }
    });
}

/// Texte d'un champ JSON (chaîne ou nombre).
fn text(v: &Value) -> String {
    v.as_str().map(String::from).or_else(|| v.as_u64().map(|n| n.to_string())).unwrap_or_default()
}

fn push_restream(state: &Mutex<State>, v: Value) {
    let s = |k: &str| text(&v[k]);
    let platform = s("platform");
    if let Some(action) = v["action"].as_str() {
        let a = match action {
            "delete-message" => "delete",
            "clear-user" => "user",
            "clear-all" => "clear",
            _ => return,
        };
        let user = if v["userLogin"].is_string() { s("userLogin") } else { s("username") };
        return moderate(state, &platform, a, &s("messageId"), &s("userId"), &user);
    }
    // Restream rejoue un court historique à chaque (re)connexion : pas de doublons.
    let ts = v["timestamp"].as_f64().unwrap_or(0.0);
    let historical = v["historical"].as_bool() == Some(true);
    let divider = v["divider"].as_bool() == Some(true);
    {
        let mut st = state.lock();
        if historical {
            if ts > 0.0 && ts <= st.last_ts {
                return;
            }
            st.replayed = true;
        }
        if divider {
            if !st.replayed {
                return;
            }
            st.replayed = false;
        }
        if ts > 0.0 && !divider && v["system"].as_bool() != Some(true) {
            st.last_ts = st.last_ts.max(ts);
        }
    }
    if divider {
        let mut m = message(&platform);
        m.divider = true;
        m.system = true;
        m.parts = vec![Part::Text(s("message"))];
        return add(state, m);
    }
    let emotes: Vec<(String, usize, usize)> = v["emotes"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| Some((text(&e["id"]), e["start"].as_u64()? as usize, e["end"].as_u64()? as usize))).collect())
        .unwrap_or_default();
    let badges = v["badges"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| {
                    let kind = b["type"].as_str().or(b.as_str())?;
                    let mut badge = Badge::of(kind);
                    badge.title = b["title"].as_str().unwrap_or("").into();
                    badge.image = b["imageUrl"].as_str().filter(|u| !u.is_empty()).map(String::from);
                    badge.count = b["count"].as_u64();
                    Some(badge)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut m = message(&platform);
    m.id = if v["id"].is_null() { s("messageId") } else { s("id") };
    m.user_id = s("userId");
    m.username = s("username");
    m.color = v["color"].as_str().and_then(hex);
    m.parts = parse_parts(v["message"].as_str().unwrap_or(""), &emotes);
    m.badges = badges;
    m.system = v["system"].as_bool().unwrap_or(false);
    m.mentioned = v["mentioned"].as_bool().unwrap_or(false);
    if historical {
        m.at = None;
    }
    add(state, m);
}

/// Images officielles des badges Twitch (globaux et de la chaîne), sans compte.
fn twitch_badge_map(login: &str) -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    let mut load = |url: &str| {
        let Ok(mut r) = ureq::get(url).header("Accept", "application/json").call() else { return };
        let Ok(list) = r.body_mut().read_to_string().map(|t| serde_json::from_str::<Value>(&t).unwrap_or_default()) else { return };
        for set in list.as_array().into_iter().flatten() {
            for v in set["versions"].as_array().into_iter().flatten() {
                let url = v["image_url_2x"].as_str().or(v["image_url_1x"].as_str()).unwrap_or("");
                if !url.is_empty() {
                    map.insert(format!("{}/{}", text(&set["set_id"]), text(&v["id"])), (url.to_string(), v["title"].as_str().unwrap_or("").to_string()));
                }
            }
        }
    };
    load("https://api.ivr.fi/v2/twitch/badges/global");
    load(&format!("https://api.ivr.fi/v2/twitch/badges/channel?login={login}"));
    map
}

fn resolve_twitch_badges(m: &mut Message, map: &HashMap<String, (String, String)>) {
    for b in &mut m.badges {
        if b.image.is_some() {
            continue;
        }
        let version = b.title.clone(); // version Twitch gardée dans le titre en attendant l'image
        let found = map.get(&format!("{}/{version}", b.kind)).or_else(|| map.get(&format!("{}/1", b.kind))).or_else(|| map.get(&format!("{}/0", b.kind)));
        if let Some((url, title)) = found {
            b.image = Some(url.clone());
            b.title = title.clone();
        }
    }
}

/// Chat Twitch anonyme (IRC sur WebSocket, comme la version Electron).
fn twitch_irc(channel: &str, state: &Mutex<State>, stop: &AtomicBool, version: &AtomicU64) {
    let set = |s: &str, e: Option<String>| {
        state.lock().status.insert("twitch".into(), (s.into(), e));
        version.fetch_add(1, Ordering::Relaxed);
    };
    set("connecting", None);
    let Ok((mut ws, _)) = tungstenite::connect("wss://irc-ws.chat.twitch.tv:443") else {
        return set("error", Some("connexion impossible".into()));
    };
    if let tungstenite::stream::MaybeTlsStream::Rustls(s) = ws.get_mut() {
        let _ = s.get_mut().set_read_timeout(Some(Duration::from_millis(500)));
    }
    let nick = format!("justinfan{}", 10000 + std::process::id() % 80000);
    for line in ["CAP REQ :twitch.tv/tags twitch.tv/commands".to_string(), "PASS SCHMOOPIIE".into(), format!("NICK {nick}"), format!("JOIN #{channel}")] {
        if ws.send(tungstenite::Message::text(line)).is_err() {
            return;
        }
    }
    while !stop.load(Ordering::Relaxed) {
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => return,
        };
        let tungstenite::Message::Text(text) = msg else { continue };
        for line in text.as_str().split("\r\n").filter(|l| !l.is_empty()) {
            if line.starts_with("PING") {
                let _ = ws.send(tungstenite::Message::text("PONG :tmi.twitch.tv"));
                continue;
            }
            handle_irc(line, state);
            if line.contains(" ROOMSTATE #") || line.contains(" JOIN #") {
                set("connected", None);
            } else {
                version.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn handle_irc(line: &str, state: &Mutex<State>) {
    let mut tags = BTreeMap::new();
    let mut rest = line;
    if let Some(stripped) = rest.strip_prefix('@')
        && let Some((t, r)) = stripped.split_once(' ')
    {
        for kv in t.split(';') {
            if let Some((k, v)) = kv.split_once('=') {
                tags.insert(k.to_string(), v.to_string());
            }
        }
        rest = r;
    }
    let tag = |k: &str| tags.get(k).cloned().unwrap_or_default();
    if let Some(i) = rest.find(" PRIVMSG #") {
        let user = rest[1..].split('!').next().unwrap_or("").to_string();
        let Some(j) = rest[i + 10..].find(" :") else { return };
        let mut text = rest[i + 10 + j + 2..].to_string();
        let action = text.starts_with("\u{1}ACTION ") && text.ends_with('\u{1}');
        if action {
            text = text[8..text.len() - 1].to_string();
        }
        let badges: Vec<Badge> = tag("badges")
            .split(',')
            .filter(|b| !b.is_empty())
            .map(|b| {
                let (kind, v) = b.split_once('/').unwrap_or((b, ""));
                let mut badge = Badge::of(kind);
                badge.title = v.to_string();
                if kind == "subscriber" || kind == "bits" {
                    badge.count = v.parse().ok();
                }
                badge
            })
            .collect();
        let mut emotes = Vec::new();
        if !action {
            for group in tag("emotes").split('/').filter(|g| !g.is_empty()) {
                let Some((id, ranges)) = group.split_once(':') else { continue };
                for r in ranges.split(',') {
                    if let Some((a, b)) = r.split_once('-')
                        && let (Ok(a), Ok(b)) = (a.parse(), b.parse())
                    {
                        emotes.push((id.to_string(), a, b));
                    }
                }
            }
        }
        let name = tag("display-name").replace("\\s", " ");
        let mut m = message("twitch");
        m.id = tag("id");
        m.user_id = tag("user-id");
        m.username = if name.is_empty() { user } else { name };
        m.color = hex(&tag("color"));
        m.parts = parse_parts(&text, &emotes);
        m.badges = badges;
        add(state, m);
    } else if rest.starts_with(":tmi.twitch.tv CLEARMSG #") {
        moderate(state, "twitch", "delete", &tag("target-msg-id"), "", "");
    } else if rest.starts_with(":tmi.twitch.tv CLEARCHAT #") {
        match rest.split(" :").nth(1) {
            Some(user) => moderate(state, "twitch", "user", "", &tag("target-user-id"), user),
            None => moderate(state, "twitch", "clear", "", "", ""),
        }
    }
}

/// Script injecté dans la page de chat Kick : relaie les trames de chat reçues par ses WebSocket.
const KICK_HOOK: &str = r#"(() => {
  const Native = window.WebSocket;
  const post = (m) => { try { window.ipc.postMessage(JSON.stringify(m)); } catch (e) {} };
  function Observed(url, protocols) {
    const ws = protocols === undefined ? new Native(url) : new Native(url, protocols);
    post({ t: 'open' });
    ws.addEventListener('message', (e) => {
      if (typeof e.data === 'string' && /ChatMessageEvent|MessageDeletedEvent|UserBannedEvent|ChatroomClearEvent/.test(e.data)) post({ t: 'frame', data: e.data });
    });
    ws.addEventListener('close', () => post({ t: 'close' }));
    return ws;
  }
  Observed.prototype = Native.prototype;
  Object.assign(Observed, Native);
  window.WebSocket = Observed;
})();"#;

/// Chat Kick sans compte : la page de chat « popout » officielle de la chaîne
/// (celle que Kick propose pour OBS), ouverte hors de l'écran.
fn kick_page(slug: &str, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>, version: Arc<AtomicU64>) {
    let set_status = {
        let (state, version) = (state.clone(), version.clone());
        move |s: &str, e: Option<String>| {
            state.lock().status.insert("kick".into(), (s.into(), e));
            version.fetch_add(1, Ordering::Relaxed);
        }
    };
    set_status("connecting", None);
    let on_message = {
        let set_status = set_status.clone();
        move |raw: String| {
            let Ok(v) = serde_json::from_str::<Value>(&raw) else { return };
            match v["t"].as_str() {
                Some("open") => set_status("connected", None),
                Some("close") => set_status("connecting", Some("reconnexion…".into())),
                Some("frame") => {
                    kick_frame(&state, v["data"].as_str().unwrap_or(""));
                    version.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    };
    let url = format!("https://kick.com/popout/{slug}/chat");
    if let Err(e) = crate::capture::web_page(&url, KICK_HOOK, "kick-chat", Box::new(on_message), stop) {
        set_status("error", Some(e));
    }
}

fn kick_frame(state: &Mutex<State>, data: &str) {
    for line in data.split('\n') {
        let Ok(frame) = serde_json::from_str::<Value>(line) else { continue };
        let public = &frame["push"]["pub"]["data"];
        let raw = match &public["data"] {
            Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
            v => v.clone(),
        };
        if raw.is_null() {
            continue;
        }
        let event = public["event"].as_str().unwrap_or("");
        let event = event.rsplit('\\').next().unwrap_or(event);
        match event {
            "ChatMessageEvent" if raw["type"].as_str() != Some("reply-deleted") => {
                let id = text(&raw["id"]);
                {
                    let mut st = state.lock();
                    if !id.is_empty() && !st.seen.insert(id.clone()) {
                        continue;
                    }
                    if st.seen.len() > 1000 {
                        st.seen.clear();
                    }
                }
                let sender = &raw["sender"];
                let identity = &sender["identity"];
                // badges_v2 fournit les images officielles ; les autres badges sont dessinés.
                let mut badges: Vec<Badge> = identity["badges_v2"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| {
                        let name = b["name"].as_str().unwrap_or("");
                        let mut badge = Badge::of(name);
                        badge.title = name.into();
                        badge.image = b["image_url"].as_str().filter(|u| !u.is_empty()).map(String::from);
                        badge
                    })
                    .collect();
                let known: HashSet<String> = badges.iter().map(|b| b.kind.clone()).collect();
                let v1: Vec<Badge> = identity["badges"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| {
                        let mut badge = Badge::of(b["type"].as_str().unwrap_or(""));
                        badge.count = b["count"].as_u64();
                        badge
                    })
                    .filter(|b| !known.contains(&b.kind))
                    .collect();
                badges.splice(0..0, v1);
                let mut m = message("kick");
                m.id = id;
                m.user_id = text(&sender["id"]);
                m.username = sender["username"].as_str().unwrap_or("inconnu").into();
                m.color = identity["color"].as_str().and_then(hex);
                m.parts = parse_parts(raw["content"].as_str().unwrap_or(""), &[]);
                m.badges = badges;
                add(state, m);
            }
            "MessageDeletedEvent" => {
                let id = if raw["message"]["id"].is_null() { text(&raw["message_id"]) } else { text(&raw["message"]["id"]) };
                moderate(state, "kick", "delete", &id, "", "");
            }
            "UserBannedEvent" => moderate(state, "kick", "user", "", &text(&raw["user"]["id"]), raw["user"]["username"].as_str().unwrap_or("")),
            "ChatroomClearEvent" => moderate(state, "kick", "clear", "", "", ""),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lit_un_message_twitch() {
        let st = Mutex::new(State::default());
        handle_irc(
            "@badges=moderator/1,subscriber/12;color=#FF0000;display-name=Bloop;emotes=25:6-10;id=abc;user-id=42 :bloop!bloop@bloop.tmi.twitch.tv PRIVMSG #chan :Salut Kappa !",
            &st,
        );
        let m = st.lock().messages[0].clone();
        assert_eq!(m.username, "Bloop");
        assert_eq!(m.badges.iter().map(|b| b.kind.as_str()).collect::<Vec<_>>(), vec!["moderator", "subscriber"]);
        assert_eq!(m.badges[1].count, Some(12));
        assert_eq!(m.color, Some([255, 0, 0]));
        assert!(matches!(&m.parts[1], Part::Emote { name, .. } if name == "Kappa"));
        handle_irc("@target-msg-id=abc :tmi.twitch.tv CLEARMSG #chan :Salut", &st);
        assert!(st.lock().messages.is_empty());
    }
    #[test]
    fn lit_un_message_kick() {
        let st = Mutex::new(State::default());
        let inner = r##"{"id":"m1","content":"gg [emote:37226:KEKW]","type":"message","sender":{"id":7,"username":"Bob","identity":{"color":"#00FF00","badges":[{"type":"subscriber","count":3}],"badges_v2":[{"name":"og","image_url":"https://x/og.png"}]}}}"##;
        let frame = serde_json::json!({ "push": { "pub": { "data": { "event": "App\\Events\\ChatMessageEvent", "data": inner } } } });
        kick_frame(&st, &frame.to_string());
        kick_frame(&st, &frame.to_string()); // doublon ignoré
        let m = st.lock().messages[0].clone();
        assert_eq!(st.lock().messages.len(), 1);
        assert_eq!(m.username, "Bob");
        assert_eq!(m.user_id, "7");
        assert_eq!(m.badges.len(), 2);
        assert_eq!(m.badges[1].image.as_deref(), Some("https://x/og.png"));
        let del = serde_json::json!({ "push": { "pub": { "data": { "event": "App\\Events\\MessageDeletedEvent", "data": { "message": { "id": "m1" } } } } } });
        kick_frame(&st, &del.to_string());
        assert!(st.lock().messages.is_empty());
    }
    #[test]
    fn adresse_kick() {
        assert_eq!(kick_slug("https://kick.com/Ma_Chaine?x=1"), "ma-chaine");
    }
    #[test]
    fn lit_les_emotes_kick() {
        let p = parse_parts("gg [emote:37226:KEKW] bien", &[]);
        assert_eq!(p.len(), 3);
    }
}
