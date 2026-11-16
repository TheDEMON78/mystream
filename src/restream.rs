//! Client de l'application Restream (BloopStudio), comme dans la version
//! Electron : adresse d'ingestion RTMP, état des relais, titre, catégorie et
//! spectateurs des chaînes, et chat agrégé (flux SSE).

use crate::model::RestreamSettings;
use serde_json::Value;
use std::io::BufRead;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

fn base(cfg: &RestreamSettings) -> Result<String, String> {
    let raw = cfg.url.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err("Adresse de Restream non configurée (Paramètres > Restream).".into());
    }
    Ok(if raw.starts_with("http://") || raw.starts_with("https://") { raw.to_string() } else { format!("http://{raw}") })
}

fn agent(cfg: &RestreamSettings, timeout: Option<Duration>) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().disable_verification(cfg.allow_self_signed).build();
    ureq::Agent::config_builder().timeout_global(timeout).tls_config(tls).http_status_as_error(false).build().into()
}

fn auth(cfg: &RestreamSettings) -> Option<String> {
    use base64::Engine as _;
    (!cfg.password.is_empty()).then(|| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("mystream:{}", cfg.password))))
}

fn friendly(e: ureq::Error) -> String {
    let s = e.to_string();
    if s.contains("refused") || s.contains("ConnectionRefused") {
        "Restream est injoignable : vérifiez que l'application est lancée et que l'adresse est correcte.".into()
    } else if s.contains("timed out") || s.contains("Timeout") {
        "Restream ne répond pas.".into()
    } else {
        s
    }
}

fn request(cfg: &RestreamSettings, path: &str, timeout_s: u64, body: Option<&Value>) -> Result<Value, String> {
    let url = format!("{}{path}", base(cfg)?);
    let a = agent(cfg, Some(Duration::from_secs(timeout_s)));
    let resp = match body {
        Some(b) => {
            let mut r = a.post(&url).header("Accept", "application/json").header("Content-Type", "application/json");
            if let Some(h) = auth(cfg) {
                r = r.header("Authorization", h);
            }
            r.send(b.to_string())
        }
        None => {
            let mut r = a.get(&url).header("Accept", "application/json");
            if let Some(h) = auth(cfg) {
                r = r.header("Authorization", h);
            }
            r.call()
        }
    };
    let mut resp = resp.map_err(friendly)?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 401 {
        return Err("Mot de passe Restream incorrect.".into());
    }
    let json: Option<Value> = serde_json::from_str(&text).ok();
    if status >= 400 {
        return Err(json.as_ref().and_then(|j| j["error"].as_str()).map(String::from).unwrap_or(format!("Restream a répondu {status}.")));
    }
    json.ok_or_else(|| "Réponse inattendue : est-ce bien l'adresse de Restream ?".into())
}

/// Adresse RTMP complète vers laquelle envoyer le flux, et la clé (à masquer dans les journaux).
pub fn ingest_url(cfg: &RestreamSettings) -> Result<(String, String), String> {
    let conf = request(cfg, "/api/config", 5, None)?;
    let key = conf["ingestKey"].as_str().filter(|k| !k.is_empty()).ok_or("Restream n'a pas fourni de clé d'ingestion.")?.to_string();
    let over = cfg.rtmp_url.trim().trim_end_matches('/');
    if !over.is_empty() {
        if !over.starts_with("rtmp://") && !over.starts_with("rtmps://") {
            return Err("L'adresse RTMP doit commencer par rtmp://".into());
        }
        let b = if over.to_lowercase().ends_with("/live") { over.to_string() } else { format!("{over}/live") };
        return Ok((format!("{b}/{key}"), key));
    }
    let b = base(cfg)?;
    let host = b.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
    // « localhost » peut se résoudre en IPv6 alors que le serveur RTMP n'écoute qu'en IPv4.
    let host = if host.eq_ignore_ascii_case("localhost") { "127.0.0.1".to_string() } else { host };
    let port = conf["rtmpPort"].as_u64().unwrap_or(1935);
    Ok((format!("rtmp://{host}:{port}/live/{key}"), key))
}

/// Vérifie que le serveur RTMP accepte une connexion avant de lancer FFmpeg.
pub fn check_rtmp(url: &str) -> Result<(), String> {
    let rest = url.split("://").nth(1).ok_or_else(|| format!("Adresse RTMP invalide : {url}"))?;
    let hostport = rest.split('/').next().unwrap_or("");
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h.trim_matches(['[', ']']).to_string(), p.parse::<u16>().unwrap()),
        _ => (hostport.to_string(), if url.starts_with("rtmps") { 443 } else { 1935 }),
    };
    let addr =
        (host.as_str(), port).to_socket_addrs().map_err(|_| format!("Adresse du serveur RTMP introuvable : {host}."))?.next().ok_or("Adresse introuvable")?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map(|_| ()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::ConnectionRefused {
            format!("Le serveur RTMP {host}:{port} refuse la connexion : Restream (ou le serveur) n'est pas lancé, ou son serveur RTMP n'a pas démarré.")
        } else {
            format!("Le serveur RTMP {host}:{port} ne répond pas. Le port {port} est sans doute bloqué par un pare-feu, ou l'adresse n'est pas la bonne.")
        }
    })
}

/// Titre, catégorie et spectateurs d'une plateforme, lus par Restream.
#[derive(Clone, Debug, Default)]
pub struct PlatformInfo {
    pub connected: bool,
    pub title: String,
    pub category: String,
    pub viewers: Option<u64>,
    pub error: Option<String>,
}

pub const PLATFORMS: [&str; 2] = ["twitch", "kick"];

pub fn live_info(cfg: &RestreamSettings) -> Vec<(String, PlatformInfo)> {
    PLATFORMS
        .iter()
        .map(|p| {
            let info = match request(cfg, &format!("/api/{p}/status"), 8, None) {
                Ok(st) => PlatformInfo {
                    connected: st["connected"].as_bool().unwrap_or(false),
                    title: st["title"].as_str().unwrap_or("").into(),
                    category: st["category"].as_str().unwrap_or("").into(),
                    viewers: st["viewerCount"].as_u64(),
                    error: None,
                },
                Err(e) => PlatformInfo { error: Some(e), ..Default::default() },
            };
            (p.to_string(), info)
        })
        .collect()
}

/// Catégories communes aux plateformes connectées (recherche à partir de 3 lettres).
pub fn categories(cfg: &RestreamSettings, query: &str) -> Result<Vec<String>, String> {
    let q: String = query.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let r = request(cfg, &format!("/api/unified/categories?query={q}"), 8, None)?;
    Ok(r["categories"]
        .as_array()
        .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from).or_else(|| c["name"].as_str().map(String::from))).collect())
        .unwrap_or_default())
}

/// Met à jour le titre et la catégorie sur toutes les plateformes connectées.
pub fn update_info(cfg: &RestreamSettings, title: &str, category: &str) -> Result<Value, String> {
    let body = serde_json::json!({ "title": title, "category": category });
    request(cfg, "/api/unified/update", 10, Some(&body)).map(|r| r["results"].clone())
}

/// Lit le flux SSE du chat agrégé ; `on_event` reçoit chaque message JSON.
/// Renvoie quand le flux se termine (l'appelant reconnecte).
pub fn chat_stream(
    cfg: &RestreamSettings,
    stop: &std::sync::atomic::AtomicBool,
    mut on_status: impl FnMut(&str, Option<String>),
    mut on_event: impl FnMut(Value),
) {
    let url = match base(cfg) {
        Ok(b) => format!("{b}/api/chat/stream"),
        Err(e) => return on_status("error", Some(e)),
    };
    on_status("connecting", None);
    let mut r = agent(cfg, None).get(&url).header("Accept", "text/event-stream");
    if let Some(h) = auth(cfg) {
        r = r.header("Authorization", h);
    }
    let resp = match r.call() {
        Ok(r) => r,
        Err(e) => return on_status("error", Some(friendly(e))),
    };
    if resp.status().as_u16() != 200 {
        let code = resp.status().as_u16();
        return on_status("error", Some(if code == 401 { "Mot de passe Restream incorrect.".into() } else { format!("Restream a répondu {code}.") }));
    }
    on_status("connected", None);
    let reader = std::io::BufReader::new(resp.into_body().into_reader());
    let mut data = String::new();
    for line in reader.lines() {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let Ok(line) = line else { break };
        if line.is_empty() {
            if !data.is_empty()
                && let Ok(v) = serde_json::from_str::<Value>(&data)
            {
                on_event(v);
            }
            data.clear();
        } else if let Some(d) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(d.trim_start());
        }
    }
    on_status("error", Some("Connexion au chat perdue, nouvelle tentative…".into()));
}
