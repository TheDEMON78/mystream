//! Pilotage de l'application : requêtes obs-websocket (Stream Deck…),
//! événements envoyés aux clients, et raccourcis clavier globaux.

use super::App;
use crate::audio::{DESKTOP, MIC};
use crate::model::HOTKEY_ACTIONS;
use crate::output::Kind;
use crate::remote::Remote;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde_json::{Value, json};
use std::str::FromStr;

type Res = Result<Value, (u16, String)>;

fn not_found(what: &str) -> (u16, String) {
    (600, format!("{what} introuvable"))
}

/// État suivi pour envoyer les événements obs-websocket quand il change.
#[derive(Clone, PartialEq, Default)]
pub struct Snapshot {
    scenes: Vec<String>,
    program: String,
    preview: String,
    studio: bool,
    stream: bool,
    record: bool,
    mutes: Vec<(String, bool)>,
    items: Vec<(String, u64, bool)>,
}

impl App {
    /// Démarre, redémarre ou arrête le serveur selon les paramètres.
    pub(super) fn apply_remote(&mut self) {
        let r = &self.project.settings.remote;
        self.remote = None;
        if !r.enabled {
            return;
        }
        let pw = (!r.password.is_empty()).then(|| r.password.clone());
        match Remote::start(r.port, r.lan, pw) {
            Ok(server) => self.remote = Some(server),
            Err(e) => self.notify(format!("Contrôle à distance indisponible (port {} : {e}). Changez de port dans les paramètres.", r.port), true),
        }
    }

    /// Entrées audio vues par les clients : nom, identifiant de piste.
    fn inputs(&self) -> Vec<(String, u64)> {
        let mut list = vec![("Micro".to_string(), MIC), ("Son du bureau".to_string(), DESKTOP)];
        list.extend(self.project.sources.iter().filter(|s| s.kind.has_audio()).map(|s| (s.name.clone(), s.id)));
        list
    }

    fn input_id(&self, d: &Value) -> Result<u64, (u16, String)> {
        let name = d["inputName"].as_str();
        let uuid = d["inputUuid"].as_str();
        self.inputs()
            .into_iter()
            .find(|(n, id)| name == Some(n.as_str()) || uuid == Some(id.to_string().as_str()))
            .map(|(_, id)| id)
            .ok_or_else(|| not_found("Entrée audio"))
    }

    fn gain(&mut self, id: u64) -> (&mut f32, &mut bool) {
        if id == MIC {
            let a = &mut self.project.settings.audio;
            return (&mut a.mic_volume, &mut a.mic_muted);
        }
        if id == DESKTOP {
            let a = &mut self.project.settings.audio;
            return (&mut a.desktop_volume, &mut a.desktop_muted);
        }
        let s = self.project.sources.iter_mut().find(|s| s.id == id).expect("source");
        (&mut s.volume, &mut s.muted)
    }

    fn scene_by(&self, d: &Value) -> Result<u64, (u16, String)> {
        let name = d["sceneName"].as_str();
        let uuid = d["sceneUuid"].as_str();
        self.project
            .scenes
            .iter()
            .find(|s| name == Some(s.name.as_str()) || uuid == Some(s.id.to_string().as_str()))
            .map(|s| s.id)
            .ok_or_else(|| not_found("Scène"))
    }

    fn scene_name(&self, id: u64) -> String {
        self.project.scene(id).map(|s| s.name.clone()).unwrap_or_default()
    }

    fn output_status(&self, kind: Kind) -> Value {
        let outputs = self.shared.outputs.lock();
        let out = outputs.get(&crate::engine::key(kind));
        let ms = out.map(|o| o.started.elapsed().as_millis() as u64).unwrap_or(0);
        let t = ms / 1000;
        json!({
            "outputActive": out.is_some(),
            "outputReconnecting": false,
            "outputDuration": ms,
            "outputTimecode": format!("{:02}:{:02}:{:02}.000", t / 3600, t / 60 % 60, t % 60),
            "outputBytes": 0,
            "outputSkippedFrames": out.map(|o| o.stats.lock().drops).unwrap_or(0),
            "outputTotalFrames": ms * self.project.settings.fps as u64 / 1000,
        })
    }

    fn call(&mut self, kind: &str, d: &Value) -> Res {
        let ok = Ok(Value::Null);
        match kind {
            "GetVersion" => Ok(json!({
                "obsVersion": "30.0.0",
                "obsWebSocketVersion": "5.5.0",
                "rpcVersion": 1,
                "platform": std::env::consts::OS,
                "platformDescription": format!("MyStream natif {}", env!("CARGO_PKG_VERSION")),
                "availableRequests": ["GetVersion", "GetStats", "GetSceneList", "GetCurrentProgramScene", "SetCurrentProgramScene", "GetCurrentPreviewScene", "SetCurrentPreviewScene", "GetStudioModeEnabled", "SetStudioModeEnabled", "TriggerStudioModeTransition", "GetInputList", "GetInputMute", "SetInputMute", "ToggleInputMute", "GetInputVolume", "SetInputVolume", "GetSpecialInputs", "GetSceneItemList", "GetSceneItemId", "GetSceneItemEnabled", "SetSceneItemEnabled", "GetStreamStatus", "StartStream", "StopStream", "ToggleStream", "GetRecordStatus", "StartRecord", "StopRecord", "ToggleRecord"],
                "supportedImageFormats": [],
            })),
            "GetStats" => {
                let st = self.shared.stats.lock().clone();
                Ok(
                    json!({ "activeFps": st.fps, "averageFrameRenderTime": st.render_ms, "renderSkippedFrames": st.lagged, "renderTotalFrames": 0, "outputSkippedFrames": st.dropped, "outputTotalFrames": 0, "cpuUsage": 0, "memoryUsage": 0 }),
                )
            }
            "GetSceneList" => {
                let n = self.project.scenes.len();
                // OBS liste les scènes de la dernière à la première.
                let scenes: Vec<Value> = self
                    .project
                    .scenes
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(i, s)| json!({ "sceneName": s.name, "sceneUuid": s.id.to_string(), "sceneIndex": n - 1 - i }))
                    .collect();
                let (p, v) = (self.scene_name(self.project.current), self.scene_name(self.project.preview));
                Ok(
                    json!({ "currentProgramSceneName": p, "currentProgramSceneUuid": self.project.current.to_string(), "currentPreviewSceneName": if self.project.settings.studio { Value::from(v) } else { Value::Null }, "scenes": scenes }),
                )
            }
            "GetCurrentProgramScene" => Ok(
                json!({ "sceneName": self.scene_name(self.project.current), "currentProgramSceneName": self.scene_name(self.project.current), "sceneUuid": self.project.current.to_string() }),
            ),
            "SetCurrentProgramScene" => {
                self.project.current = self.scene_by(d)?;
                self.touch();
                ok
            }
            "GetCurrentPreviewScene" => {
                if !self.project.settings.studio {
                    return Err((506, "Le mode studio n'est pas activé".into()));
                }
                Ok(
                    json!({ "sceneName": self.scene_name(self.project.preview), "currentPreviewSceneName": self.scene_name(self.project.preview), "sceneUuid": self.project.preview.to_string() }),
                )
            }
            "SetCurrentPreviewScene" => {
                if !self.project.settings.studio {
                    return Err((506, "Le mode studio n'est pas activé".into()));
                }
                self.project.preview = self.scene_by(d)?;
                self.touch();
                ok
            }
            "GetStudioModeEnabled" => Ok(json!({ "studioModeEnabled": self.project.settings.studio })),
            "SetStudioModeEnabled" => {
                self.project.settings.studio = d["studioModeEnabled"].as_bool().unwrap_or(false);
                self.project.preview = self.project.current;
                self.touch();
                ok
            }
            "TriggerStudioModeTransition" => {
                self.transition();
                ok
            }
            "GetInputList" => {
                let inputs: Vec<Value> = self.inputs().into_iter().map(|(n, id)| json!({ "inputName": n, "inputUuid": id.to_string(), "inputKind": "wasapi_input_capture", "unversionedInputKind": "wasapi_input_capture" })).collect();
                Ok(json!({ "inputs": inputs }))
            }
            "GetSpecialInputs" => Ok(json!({ "desktop1": "Son du bureau", "desktop2": null, "mic1": "Micro", "mic2": null, "mic3": null, "mic4": null })),
            "GetInputMute" => {
                let id = self.input_id(d)?;
                Ok(json!({ "inputMuted": *self.gain(id).1 }))
            }
            "SetInputMute" | "ToggleInputMute" => {
                let id = self.input_id(d)?;
                let (_, muted) = self.gain(id);
                *muted = if kind == "ToggleInputMute" { !*muted } else { d["inputMuted"].as_bool().unwrap_or(*muted) };
                let m = *muted;
                self.touch();
                Ok(if kind == "ToggleInputMute" { json!({ "inputMuted": m }) } else { Value::Null })
            }
            "GetInputVolume" => {
                let id = self.input_id(d)?;
                let v = *self.gain(id).0;
                Ok(json!({ "inputVolumeMul": v, "inputVolumeDb": if v > 0.0 { 20.0 * v.log10() } else { -100.0 } }))
            }
            "SetInputVolume" => {
                let id = self.input_id(d)?;
                let mul = d["inputVolumeMul"].as_f64().map(|m| m as f32).or_else(|| d["inputVolumeDb"].as_f64().map(|db| 10f32.powf(db as f32 / 20.0)));
                let (vol, _) = self.gain(id);
                if let Some(m) = mul {
                    *vol = m.clamp(0.0, 2.0);
                }
                self.touch();
                ok
            }
            "GetSceneItemList" => {
                let id = self.scene_by(d)?;
                let scene = self.project.scene(id).ok_or_else(|| not_found("Scène"))?;
                let n = scene.items.len();
                let items: Vec<Value> = scene
                    .items
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(i, it)| {
                        let name = self.project.source(it.source).map(|s| s.name.clone()).unwrap_or_default();
                        json!({ "sceneItemId": it.id, "sourceName": name, "sourceUuid": it.source.to_string(), "sceneItemEnabled": it.visible, "sceneItemIndex": n - 1 - i, "inputKind": Value::Null, "isGroup": false })
                    })
                    .collect();
                Ok(json!({ "sceneItems": items }))
            }
            "GetSceneItemId" => {
                let id = self.scene_by(d)?;
                let name = d["sourceName"].as_str().unwrap_or("");
                let scene = self.project.scene(id).ok_or_else(|| not_found("Scène"))?;
                scene
                    .items
                    .iter()
                    .find(|it| self.project.source(it.source).is_some_and(|s| s.name == name))
                    .map(|it| json!({ "sceneItemId": it.id }))
                    .ok_or_else(|| not_found("Source"))
            }
            "GetSceneItemEnabled" | "SetSceneItemEnabled" => {
                let id = self.scene_by(d)?;
                let item_id = d["sceneItemId"].as_u64().ok_or((300, "sceneItemId manquant".to_string()))?;
                let set = d["sceneItemEnabled"].as_bool();
                let scene = self.project.scene_mut(id).ok_or_else(|| not_found("Scène"))?;
                let item = scene.items.iter_mut().find(|i| i.id == item_id).ok_or_else(|| not_found("Élément"))?;
                if kind == "GetSceneItemEnabled" {
                    return Ok(json!({ "sceneItemEnabled": item.visible }));
                }
                item.visible = set.unwrap_or(item.visible);
                self.touch();
                ok
            }
            "GetStreamStatus" => Ok(self.output_status(Kind::Stream)),
            "GetRecordStatus" => {
                let mut v = self.output_status(Kind::Record);
                v["outputPaused"] = json!(false);
                Ok(v)
            }
            "StartStream" | "StopStream" | "ToggleStream" | "StartRecord" | "StopRecord" | "ToggleRecord" => {
                let k = if kind.ends_with("Stream") { Kind::Stream } else { Kind::Record };
                let active = self.live(k);
                // « StartStream » -> « Start », « ToggleRecord » -> « Toggle »…
                let want = match &kind[..kind.len() - 6] {
                    "Start" => true,
                    "Stop" => false,
                    _ => !active,
                };
                if want == active {
                    return Err((if active { 500 } else { 501 }, if active { "Déjà en cours".into() } else { "Pas en cours".into() }));
                }
                self.toggle_output(k);
                Ok(if kind.starts_with("Toggle") { json!({ "outputActive": want }) } else { Value::Null })
            }
            _ => Err((204, format!("Requête « {kind} » non prise en charge par MyStream"))),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            scenes: self.project.scenes.iter().map(|s| s.name.clone()).collect(),
            program: self.scene_name(self.project.current),
            preview: self.scene_name(self.project.preview),
            studio: self.project.settings.studio,
            stream: self.live(Kind::Stream),
            record: self.live(Kind::Record),
            mutes: {
                let a = &self.project.settings.audio;
                let mut m = vec![("Micro".to_string(), a.mic_muted), ("Son du bureau".to_string(), a.desktop_muted)];
                m.extend(self.project.sources.iter().filter(|s| s.kind.has_audio()).map(|s| (s.name.clone(), s.muted)));
                m
            },
            items: self.project.scenes.iter().flat_map(|s| s.items.iter().map(move |i| (s.name.clone(), i.id, i.visible))).collect(),
        }
    }

    /// Traite les requêtes reçues et envoie les événements des changements.
    pub(super) fn handle_remote(&mut self) {
        let Some(remote) = &self.remote else { return };
        let calls: Vec<_> = remote.calls.try_iter().collect();
        for c in calls {
            let res = self.call(&c.kind, &c.data);
            let _ = c.reply.send(res);
        }
        let now = self.snapshot();
        let before = std::mem::replace(&mut self.last_snapshot, now.clone());
        let Some(remote) = &self.remote else { return };
        if remote.clients() == 0 || before == Snapshot::default() {
            return;
        }
        if before.scenes != now.scenes {
            let scenes: Vec<Value> =
                now.scenes.iter().enumerate().rev().map(|(i, n)| json!({ "sceneName": n, "sceneIndex": now.scenes.len() - 1 - i })).collect();
            remote.broadcast("SceneListChanged", json!({ "scenes": scenes }));
        }
        if before.program != now.program {
            remote.broadcast("CurrentProgramSceneChanged", json!({ "sceneName": now.program }));
        }
        if before.preview != now.preview && now.studio {
            remote.broadcast("CurrentPreviewSceneChanged", json!({ "sceneName": now.preview }));
        }
        if before.studio != now.studio {
            remote.broadcast("StudioModeStateChanged", json!({ "studioModeEnabled": now.studio }));
        }
        if before.stream != now.stream {
            remote.broadcast(
                "StreamStateChanged",
                json!({ "outputActive": now.stream, "outputState": if now.stream { "OBS_WEBSOCKET_OUTPUT_STARTED" } else { "OBS_WEBSOCKET_OUTPUT_STOPPED" } }),
            );
        }
        if before.record != now.record {
            remote.broadcast(
                "RecordStateChanged",
                json!({ "outputActive": now.record, "outputState": if now.record { "OBS_WEBSOCKET_OUTPUT_STARTED" } else { "OBS_WEBSOCKET_OUTPUT_STOPPED" } }),
            );
        }
        for (name, muted) in &now.mutes {
            if before.mutes.iter().any(|(n, m)| n == name && m != muted) {
                remote.broadcast("InputMuteStateChanged", json!({ "inputName": name, "inputMuted": muted }));
            }
        }
        for (scene, id, on) in &now.items {
            if before.items.iter().any(|(s, i, o)| s == scene && i == id && o != on) {
                remote.broadcast("SceneItemEnableStateChanged", json!({ "sceneName": scene, "sceneItemId": id, "sceneItemEnabled": on }));
            }
        }
    }

    /* Raccourcis clavier globaux ------------------------------------------ */

    /// Enregistre les raccourcis des paramètres ; renvoie les combinaisons refusées.
    pub(super) fn apply_hotkeys(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.hotkey_manager.is_none() {
            match GlobalHotKeyManager::new() {
                Ok(m) => self.hotkey_manager = Some(m),
                Err(e) => {
                    errors.push(format!("raccourcis indisponibles : {e}"));
                    return errors;
                }
            }
        }
        let manager = self.hotkey_manager.as_ref().unwrap();
        let old: Vec<HotKey> = self.hotkeys.iter().map(|(h, _)| *h).collect();
        let _ = manager.unregister_all(&old);
        self.hotkeys.clear();
        for (action, _) in HOTKEY_ACTIONS {
            let Some(text) = self.project.settings.hotkeys.get(action).filter(|t| !t.trim().is_empty()) else { continue };
            match HotKey::from_str(text) {
                Ok(hk) => match manager.register(hk) {
                    Ok(()) => self.hotkeys.push((hk, action)),
                    Err(e) => errors.push(format!("{text} ({e})")),
                },
                Err(e) => errors.push(format!("{text} ({e})")),
            }
        }
        errors
    }

    pub(super) fn handle_hotkeys(&mut self) {
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if ev.state != HotKeyState::Pressed {
                continue;
            }
            let Some(&(_, action)) = self.hotkeys.iter().find(|(h, _)| h.id() == ev.id) else { continue };
            match action {
                "stream" => self.toggle_output(Kind::Stream),
                "record" => self.toggle_output(Kind::Record),
                "mic" => self.project.settings.audio.mic_muted = !self.project.settings.audio.mic_muted,
                "desktop" => self.project.settings.audio.desktop_muted = !self.project.settings.audio.desktop_muted,
                "transition" => self.transition(),
                "next" | "prev" => {
                    let scenes = &self.project.scenes;
                    let edit = self.project.edit_scene();
                    let i = scenes.iter().position(|s| s.id == edit).unwrap_or(0);
                    let n = scenes.len();
                    let j = if action == "next" { (i + 1) % n } else { (i + n - 1) % n };
                    let id = scenes[j].id;
                    if self.project.settings.studio {
                        self.project.preview = id;
                    } else {
                        self.project.current = id;
                    }
                }
                _ => {}
            }
            self.touch();
        }
    }
}
