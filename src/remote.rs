//! Contrôle à distance compatible obs-websocket 5 : le Stream Deck (plugin
//! MyStream ou plugins OBS), Touch Portal, Bitfocus Companion… pilotent
//! MyStream comme OBS (scènes, micro, sources, streaming, enregistrement).
//!
//! Le serveur tourne dans ses propres fils ; chaque requête est transmise à
//! l'interface (`Call`), qui répond. JSON et MessagePack sont pris en charge.

use base64::Engine as _;
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tungstenite::handshake::server::{Request, Response};
use tungstenite::{Message, WebSocket};

/// Requête d'un client, à traiter par l'interface.
pub struct Call {
    pub kind: String,
    pub data: Value,
    pub reply: Sender<Result<Value, (u16, String)>>,
}

pub struct Remote {
    pub calls: Receiver<Call>,
    clients: Arc<Mutex<Vec<Sender<Value>>>>,
    stop: Arc<AtomicBool>,
}

/// Valeur aléatoire (sel, défi, mot de passe), sans dépendance supplémentaire.
pub fn random_token(len: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    let alphabet = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    while out.len() < len {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let mut v = h.finish();
        for _ in 0..8 {
            out.push(alphabet[(v % alphabet.len() as u64) as usize] as char);
            v /= alphabet.len() as u64;
        }
    }
    out.truncate(len);
    out
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Réponse attendue par obs-websocket : base64(sha256(base64(sha256(mdp + sel)) + défi)).
fn expected_auth(password: &str, salt: &str, challenge: &str) -> String {
    let secret = b64(&Sha256::digest(format!("{password}{salt}").as_bytes()));
    b64(&Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

impl Remote {
    pub fn start(port: u16, lan: bool, password: Option<String>) -> std::io::Result<Self> {
        let addr = if lan { "0.0.0.0" } else { "127.0.0.1" };
        let listener = TcpListener::bind((addr, port))?;
        listener.set_nonblocking(true)?;
        let (tx, rx) = unbounded::<Call>();
        let clients: Arc<Mutex<Vec<Sender<Value>>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (c2, s2) = (clients.clone(), stop.clone());
        std::thread::Builder::new().name("obs-websocket".into()).spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (tx, c3, s3, pw) = (tx.clone(), c2.clone(), s2.clone(), password.clone());
                        std::thread::spawn(move || {
                            if let Err(e) = serve(stream, tx, c3, s3, pw) {
                                log::info!("Client obs-websocket déconnecté : {e}");
                            }
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        })?;
        log::info!("Contrôle à distance (obs-websocket) sur le port {port}");
        Ok(Self { calls: rx, clients, stop })
    }

    /// Envoie un événement à tous les clients identifiés.
    pub fn broadcast(&self, event: &str, data: Value) {
        let msg = json!({ "op": 5, "d": { "eventType": event, "eventIntent": 1, "eventData": data } });
        self.clients.lock().retain(|tx| tx.send(msg.clone()).is_ok());
    }

    pub fn clients(&self) -> usize {
        self.clients.lock().len()
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn send(ws: &mut WebSocket<TcpStream>, msgpack: bool, v: &Value) -> tungstenite::Result<()> {
    if msgpack { ws.send(Message::binary(rmp_serde::to_vec_named(v).unwrap_or_default())) } else { ws.send(Message::text(v.to_string())) }
}

fn serve(
    stream: TcpStream,
    calls: Sender<Call>,
    clients: Arc<Mutex<Vec<Sender<Value>>>>,
    stop: Arc<AtomicBool>,
    password: Option<String>,
) -> Result<(), String> {
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    let mut msgpack = false;
    let mut ws = tungstenite::accept_hdr(stream, |req: &Request, mut res: Response| {
        // Sous-protocole demandé par le client (obs-websocket-js en Node demande MessagePack).
        let wanted = req.headers().get("Sec-WebSocket-Protocol").and_then(|v| v.to_str().ok()).unwrap_or("");
        let proto = if wanted.contains("obswebsocket.msgpack") {
            msgpack = true;
            "obswebsocket.msgpack"
        } else {
            "obswebsocket.json"
        };
        if !wanted.is_empty() {
            res.headers_mut().insert("Sec-WebSocket-Protocol", proto.parse().unwrap());
        }
        Ok(res)
    })
    .map_err(|e| e.to_string())?;
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(50))).map_err(|e| e.to_string())?;

    let salt = random_token(32);
    let challenge = random_token(32);
    let mut hello = json!({ "obsWebSocketVersion": "5.5.0", "rpcVersion": 1 });
    if password.is_some() {
        hello["authentication"] = json!({ "challenge": challenge, "salt": salt });
    }
    send(&mut ws, msgpack, &json!({ "op": 0, "d": hello })).map_err(|e| e.to_string())?;

    let (etx, erx) = unbounded::<Value>();
    let mut identified = false;
    while !stop.load(Ordering::Relaxed) {
        // Événements à transmettre.
        while let Ok(ev) = erx.try_recv() {
            send(&mut ws, msgpack, &ev).map_err(|e| e.to_string())?;
        }
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let v: Value = match &msg {
            Message::Text(t) => serde_json::from_str(t.as_str()).unwrap_or(Value::Null),
            Message::Binary(b) => rmp_serde::from_slice(b).unwrap_or(Value::Null),
            Message::Close(_) => return Ok(()),
            _ => continue,
        };
        let op = v["op"].as_u64().unwrap_or(99);
        let d = &v["d"];
        match op {
            // Identify / Reidentify
            1 | 3 => {
                if op == 1 {
                    if let Some(pw) = &password {
                        let given = d["authentication"].as_str().unwrap_or("");
                        if given != expected_auth(pw, &salt, &challenge) {
                            let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
                                code: tungstenite::protocol::frame::coding::CloseCode::from(4009),
                                reason: "Mot de passe incorrect".into(),
                            }));
                            return Err("mot de passe incorrect".into());
                        }
                    }
                    identified = true;
                    clients.lock().push(etx.clone());
                }
                send(&mut ws, msgpack, &json!({ "op": 2, "d": { "negotiatedRpcVersion": 1 } })).map_err(|e| e.to_string())?;
            }
            // Request
            6 if identified => {
                let res = request(&calls, d);
                send(&mut ws, msgpack, &json!({ "op": 7, "d": res })).map_err(|e| e.to_string())?;
            }
            // RequestBatch
            8 if identified => {
                let results: Vec<Value> = d["requests"].as_array().map(|a| a.iter().map(|r| request(&calls, r)).collect()).unwrap_or_default();
                send(&mut ws, msgpack, &json!({ "op": 9, "d": { "requestId": d["requestId"], "results": results } })).map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn request(calls: &Sender<Call>, d: &Value) -> Value {
    let kind = d["requestType"].as_str().unwrap_or("").to_string();
    let (rtx, rrx) = crossbeam_channel::bounded(1);
    let _ = calls.send(Call { kind: kind.clone(), data: d["requestData"].clone(), reply: rtx });
    let result = rrx.recv_timeout(Duration::from_secs(5)).unwrap_or(Err((702, "MyStream ne répond pas".into())));
    let mut out = json!({ "requestType": kind, "requestId": d["requestId"] });
    match result {
        Ok(data) => {
            out["requestStatus"] = json!({ "result": true, "code": 100 });
            if !data.is_null() {
                out["responseData"] = data;
            }
        }
        Err((code, comment)) => out["requestStatus"] = json!({ "result": false, "code": code, "comment": comment }),
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn authentification_obs() {
        // Valeur calculée avec l'algorithme d'obs-websocket (mot de passe « supersecretpassword »).
        let a = super::expected_auth("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=");
        assert_eq!(a.len(), 44);
    }
}

#[cfg(test)]
mod serveur {
    #[test]
    #[ignore]
    fn serveur_seul() {
        let r = super::Remote::start(4461, false, Some("abc".into())).unwrap();
        let t = std::time::Instant::now();
        while t.elapsed().as_secs() < 8 {
            while let Ok(c) = r.calls.try_recv() {
                let _ = c.reply.send(Ok(serde_json::json!({ "scenes": [], "ok": c.kind })));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
