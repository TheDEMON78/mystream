//! Chat : flux agrégé de Restream (Twitch, Kick…) ou chat Twitch lu en
//! direct (anonyme, sans compte), avec badges, emotes et modération
//! (messages supprimés, viewers bannis), comme la version Electron.

use crate::model::{ChatSettings, RestreamSettings};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

const MAX_MESSAGES: usize = 200;

#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    Emote { name: String, url: String },
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
    /// Types de badges (broadcaster, moderator, vip, subscriber…).
    pub badges: Vec<String>,
    /// Message système (connexion, information).
    pub system: bool,
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

/// Libellé et couleur d'un badge (les mêmes que la version Electron).
pub fn badge(kind: &str) -> Option<(&'static str, [u8; 3])> {
    Some(match kind {
        "broadcaster" => ("Diffuseur", [0xe9, 0x19, 0x16]),
        "moderator" => ("Modo", [0x00, 0xad, 0x03]),
        "vip" => ("VIP", [0xe0, 0x05, 0xb9]),
        "subscriber" => ("Abonné", [0x82, 0x05, 0xb4]),
        "founder" => ("Fondateur", [0x3d, 0x89, 0xd8]),
        "og" => ("OG", [0x0e, 0x9e, 0x7a]),
        "verified" => ("Vérifié", [0x1f, 0xd6, 0x55]),
        "sub_gifter" => ("Cadeau abo", [0xff, 0x8c, 0x00]),
        "staff" => ("Staff", [0x3a, 0x3a, 0x3a]),
        "partner" => ("Partenaire", [0x91, 0x46, 0xff]),
        "bits" => ("Bits", [0x8a, 0x3d, 0xff]),
        "premium" => ("Prime", [0x00, 0xa3, 0xff]),
        "turbo" => ("Turbo", [0x59, 0x39, 0x9a]),
        _ => return None,
    })
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
            "twitch" if !cfg.twitch_channel.trim().is_empty() => {
                let channel = cfg.twitch_channel.trim().to_lowercase();
                let channel: String = channel.rsplit('/').next().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                let (state, stop, version) = (self.state.clone(), self.stop.clone(), self.version.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        twitch_irc(&channel, &state, &stop, &version);
                        if !stop.load(Ordering::Relaxed) {
                            state.lock().status.insert("twitch".into(), ("error".into(), Some("déconnecté, nouvelle tentative…".into())));
                            std::thread::sleep(Duration::from_secs(3));
                        }
                    }
                });
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

fn add(state: &Mutex<State>, m: Message) {
    let mut s = state.lock();
    if !m.system {
        s.total += 1;
    }
    s.messages.push_back(m);
    while s.messages.len() > MAX_MESSAGES {
        s.messages.pop_front();
    }
}

/// Retire ce que la plateforme a supprimé : un message, ceux d'un viewer, ou tout.
fn moderate(state: &Mutex<State>, platform: &str, action: &str, id: &str, user_id: &str, username: &str) {
    let name = username.to_lowercase();
    state.lock().messages.retain(|m| {
        if m.platform != platform || m.system {
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

fn push_restream(state: &Mutex<State>, v: Value) {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
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
    {
        let mut st = state.lock();
        if v["historical"].as_bool() == Some(true) {
            if ts > 0.0 && ts <= st.last_ts {
                return;
            }
            st.replayed = true;
        }
        if v["divider"].as_bool() == Some(true) {
            if !st.replayed {
                return;
            }
            st.replayed = false;
        }
        if ts > 0.0 {
            st.last_ts = st.last_ts.max(ts);
        }
    }
    if v["divider"].as_bool() == Some(true) {
        return;
    }
    let emotes: Vec<(String, usize, usize)> = v["emotes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some((
                        e["id"].as_str().map(String::from).or_else(|| e["id"].as_u64().map(|n| n.to_string()))?,
                        e["start"].as_u64()? as usize,
                        e["end"].as_u64()? as usize,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let badges = v["badges"].as_array().map(|a| a.iter().filter_map(|b| b["type"].as_str().or(b.as_str()).map(String::from)).collect()).unwrap_or_default();
    let id = if v["id"].is_string() { s("id") } else { s("messageId") };
    add(
        state,
        Message {
            id,
            user_id: s("userId"),
            platform,
            username: s("username"),
            color: v["color"].as_str().and_then(hex),
            parts: parse_parts(v["message"].as_str().unwrap_or(""), &emotes),
            badges,
            system: v["system"].as_bool().unwrap_or(false),
        },
    );
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
        let badges: Vec<String> = tag("badges").split(',').filter(|b| !b.is_empty()).map(|b| b.split('/').next().unwrap_or("").to_string()).collect();
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
        add(
            state,
            Message {
                id: tag("id"),
                user_id: tag("user-id"),
                platform: "twitch".into(),
                username: if name.is_empty() { user } else { name },
                color: hex(&tag("color")),
                parts: parse_parts(&text, &emotes),
                badges,
                system: false,
            },
        );
    } else if rest.starts_with(":tmi.twitch.tv CLEARMSG #") {
        moderate(state, "twitch", "delete", &tag("target-msg-id"), "", "");
    } else if rest.starts_with(":tmi.twitch.tv CLEARCHAT #") {
        match rest.split(" :").nth(1) {
            Some(user) => moderate(state, "twitch", "user", "", &tag("target-user-id"), user),
            None => moderate(state, "twitch", "clear", "", "", ""),
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
        assert_eq!(m.badges, vec!["moderator", "subscriber"]);
        assert_eq!(m.color, Some([255, 0, 0]));
        assert!(matches!(&m.parts[1], Part::Emote { name, .. } if name == "Kappa"));
        handle_irc("@target-msg-id=abc :tmi.twitch.tv CLEARMSG #chan :Salut", &st);
        assert!(st.lock().messages.is_empty());
    }
    #[test]
    fn lit_les_emotes_kick() {
        let p = parse_parts("gg [emote:37226:KEKW] bien", &[]);
        assert_eq!(p.len(), 3);
    }
}
