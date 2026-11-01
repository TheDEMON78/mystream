'use strict';

// Télécommande compatible obs-websocket v5 (Stream Deck, Streamer.bot, Touch
// Portal, Companion…) : traduction des requêtes d'OBS vers MyStream, et des
// changements de MyStream en événements d'OBS. Le serveur est dans le
// processus principal (src/main/remote.js).
(function () {
  const MS = window.MS;
  const store = () => MS.store;

  const CODE = { Success: 100, MissingRequestField: 300, InvalidRequestField: 400, OutputRunning: 500, OutputNotRunning: 501, StudioModeNotActive: 506, ResourceNotFound: 600, UnknownRequestType: 204 };
  const INTENT = { General: 1, Scenes: 4, Inputs: 8, Outputs: 64, SceneItems: 128, Ui: 1024 };

  class RemoteError extends Error {
    constructor(code, comment) {
      super(comment);
      this.code = code;
    }
  }

  /* Correspondances MyStream → OBS ----------------------------------------- */

  const INPUT_KINDS = {
    audio_input: 'wasapi_input_capture',
    desktop_audio: 'wasapi_output_capture',
    app_audio: 'wasapi_process_output_capture',
    game_capture: 'game_capture',
    video_capture: 'dshow_input',
    image: 'image_source',
    media: 'ffmpeg_source',
    text: 'text_gdiplus_v3',
    browser: 'browser_source',
    chat: 'browser_source',
    color: 'color_source_v3',
  };
  const inputKind = (src) => (src.type === 'display_capture' ? (src.settings.captureKind === 'window' ? 'window_capture' : 'monitor_capture') : INPUT_KINDS[src.type] || src.type);

  // Les éléments de scène d'OBS ont un numéro entier : on en attribue un, stable pendant la session.
  const itemIds = new Map();
  let nextItemId = 1;
  const itemNumber = (item) => {
    if (!itemIds.has(item.id)) itemIds.set(item.id, nextItemId++);
    return itemIds.get(item.id);
  };

  const sceneInfo = (s) => ({ sceneName: s.name, sceneUuid: s.id });

  function findScene(d) {
    const scene = store().data.scenes.find((s) => (d.sceneUuid && s.id === d.sceneUuid) || (d.sceneName != null && s.name === d.sceneName));
    if (!scene) {
      if (d.sceneUuid == null && d.sceneName == null) throw new RemoteError(CODE.MissingRequestField, 'Champ sceneName ou sceneUuid manquant.');
      throw new RemoteError(CODE.ResourceNotFound, `Scène introuvable : ${d.sceneName || d.sceneUuid}`);
    }
    return scene;
  }

  function findInput(d) {
    const src = Object.values(store().data.sources).find((s) => (d.inputUuid && s.id === d.inputUuid) || (d.inputName != null && s.name === d.inputName));
    if (!src) {
      if (d.inputUuid == null && d.inputName == null) throw new RemoteError(CODE.MissingRequestField, 'Champ inputName ou inputUuid manquant.');
      throw new RemoteError(CODE.ResourceNotFound, `Source introuvable : ${d.inputName || d.inputUuid}`);
    }
    return src;
  }

  function findItem(d) {
    const scene = findScene(d);
    const item = scene.items.find((it) => itemNumber(it) === Number(d.sceneItemId));
    if (!item) throw new RemoteError(CODE.ResourceNotFound, `Élément de scène introuvable : ${d.sceneItemId}`);
    return { scene, item };
  }

  // OBS liste les scènes de bas en haut : l'index 0 est la dernière de la liste affichée.
  function sceneList() {
    const list = store().data.scenes;
    return list.map((s, i) => ({ sceneIndex: list.length - 1 - i, ...sceneInfo(s) })).reverse();
  }

  const timecode = (ms) => `${MS.formatTime(ms)}.${String(Math.floor(ms % 1000)).padStart(3, '0')}`;

  function outputStatus(kind) {
    const out = MS.outputs.active[kind];
    const rc = kind === 'stream' && MS.outputs.reconnect;
    const start = out ? out.liveStart || out.startedAt : rc ? rc.liveStart : 0;
    const duration = start ? performance.now() - start : 0;
    const p = (out && out.progress) || {};
    const status = { outputActive: Boolean(out || rc), outputTimecode: timecode(duration), outputDuration: Math.round(duration), outputBytes: p.totalSize || 0 };
    if (kind === 'stream') Object.assign(status, { outputReconnecting: Boolean(rc), outputCongestion: p.speed ? Math.max(0, Math.min(1, 1 - p.speed)) : 0, outputSkippedFrames: p.dropFrames || 0, outputTotalFrames: Math.round((duration / 1000) * store().settings.video.fps) });
    else Object.assign(status, { outputPaused: false });
    return status;
  }

  function setMute(src, muted) {
    src.muted = Boolean(muted);
    MS.audio.updateGains();
    store().save();
    MS.emit('mixer:changed');
  }

  function setVolume(src, d) {
    let mul = d.inputVolumeMul;
    if (mul == null && d.inputVolumeDb != null) mul = Math.pow(10, Number(d.inputVolumeDb) / 20);
    if (mul == null || !Number.isFinite(Number(mul))) throw new RemoteError(CODE.MissingRequestField, 'Champ inputVolumeMul ou inputVolumeDb manquant.');
    src.volume = MS.clamp(Number(mul), 0, 20);
    MS.audio.updateGains();
    store().save();
    MS.emit('mixer:changed');
  }

  async function startOutput(kind) {
    if (MS.outputs.active[kind] || (kind === 'stream' && MS.outputs.reconnect)) throw new RemoteError(CODE.OutputRunning, 'Déjà en cours.');
    await MS.outputs.start(kind);
  }

  async function stopOutput(kind) {
    if (!MS.outputs.active[kind] && !(kind === 'stream' && MS.outputs.reconnect)) throw new RemoteError(CODE.OutputNotRunning, 'Pas en cours.');
    await MS.outputs.stop(kind);
  }

  /* Requêtes ---------------------------------------------------------------- */

  const requests = {
    GetVersion: () => ({
      obsVersion: '30.2.0',
      obsWebSocketVersion: '5.5.0',
      rpcVersion: 1,
      availableRequests: Object.keys(requests),
      supportedImageFormats: [],
      platform: MS.appInfo.platform === 'win32' ? 'windows' : MS.appInfo.platform === 'darwin' ? 'macos' : 'linux',
      platformDescription: `MyStream ${MS.appInfo.version}`,
    }),
    GetStats: () => {
      const s = MS.compositor.stats;
      const stream = MS.outputs.active.stream;
      return {
        cpuUsage: 0,
        memoryUsage: 0,
        availableDiskSpace: 0,
        activeFps: s.fps,
        averageFrameRenderTime: s.renderMs,
        renderSkippedFrames: s.lagged,
        renderTotalFrames: s.total,
        outputSkippedFrames: (stream && stream.progress && stream.progress.dropFrames) || 0,
        outputTotalFrames: s.total,
        webSocketSessionIncomingMessages: 0,
        webSocketSessionOutgoingMessages: 0,
      };
    },
    GetSceneCollectionList: () => ({ currentSceneCollectionName: 'MyStream', sceneCollections: ['MyStream'] }),
    GetProfileList: () => ({ currentProfileName: 'MyStream', profiles: ['MyStream'] }),

    // Scènes
    GetSceneList: () => {
      const program = store().currentScene();
      const studio = store().settings.ui.studio;
      const preview = studio ? store().scene(store().data.previewSceneId) : null;
      return {
        currentProgramSceneName: program ? program.name : null,
        currentProgramSceneUuid: program ? program.id : null,
        currentPreviewSceneName: preview ? preview.name : null,
        currentPreviewSceneUuid: preview ? preview.id : null,
        scenes: sceneList(),
      };
    },
    GetCurrentProgramScene: () => {
      const s = store().currentScene();
      return { ...sceneInfo(s), currentProgramSceneName: s.name, currentProgramSceneUuid: s.id };
    },
    SetCurrentProgramScene: (d) => {
      MS.compositor.switchScene(findScene(d).id);
    },
    GetCurrentPreviewScene: () => {
      if (!store().settings.ui.studio) throw new RemoteError(CODE.StudioModeNotActive, 'Le mode studio est désactivé.');
      const s = store().scene(store().data.previewSceneId);
      return { ...sceneInfo(s), currentPreviewSceneName: s.name, currentPreviewSceneUuid: s.id };
    },
    SetCurrentPreviewScene: (d) => {
      if (!store().settings.ui.studio) throw new RemoteError(CODE.StudioModeNotActive, 'Le mode studio est désactivé.');
      store().setPreviewScene(findScene(d).id);
    },
    GetStudioModeEnabled: () => ({ studioModeEnabled: Boolean(store().settings.ui.studio) }),
    SetStudioModeEnabled: (d) => store().setStudio(Boolean(d.studioModeEnabled)),
    TriggerStudioModeTransition: () => {
      if (!store().settings.ui.studio) throw new RemoteError(CODE.StudioModeNotActive, 'Le mode studio est désactivé.');
      MS.compositor.switchScene(store().data.previewSceneId);
    },
    GetSceneTransitionList: () => {
      const t = store().settings.transition;
      const name = MS.TRANSITION_LABELS[t.type] || t.type;
      return { currentSceneTransitionName: name, currentSceneTransitionKind: `${t.type}_transition`, transitions: Object.entries(MS.TRANSITION_LABELS).filter(([k]) => k !== 'default').map(([k, label]) => ({ transitionName: label, transitionKind: `${k}_transition`, transitionFixed: k === 'cut', transitionConfigurable: true })) };
    },
    GetCurrentSceneTransition: () => {
      const t = store().settings.transition;
      return { transitionName: MS.TRANSITION_LABELS[t.type] || t.type, transitionKind: `${t.type}_transition`, transitionFixed: t.type === 'cut', transitionDuration: t.type === 'cut' ? null : Number(t.duration) || 0, transitionConfigurable: true, transitionSettings: {} };
    },

    // Sources et audio
    GetInputList: (d) => ({
      inputs: Object.values(store().data.sources)
        .map((s) => ({ inputName: s.name, inputUuid: s.id, inputKind: inputKind(s), unversionedInputKind: inputKind(s).replace(/_v\d+$/, '') }))
        .filter((i) => !d.inputKind || i.inputKind === d.inputKind),
    }),
    GetSpecialInputs: () => {
      const name = (id) => (store().source(id) || {}).name || null;
      return { desktop1: name('global_desktop'), desktop2: null, mic1: name('global_mic'), mic2: null, mic3: null, mic4: null };
    },
    GetInputMute: (d) => ({ inputMuted: Boolean(findInput(d).muted) }),
    SetInputMute: (d) => {
      if (typeof d.inputMuted !== 'boolean') throw new RemoteError(CODE.MissingRequestField, 'Champ inputMuted manquant.');
      setMute(findInput(d), d.inputMuted);
    },
    ToggleInputMute: (d) => {
      const src = findInput(d);
      setMute(src, !src.muted);
      return { inputMuted: src.muted };
    },
    GetInputVolume: (d) => {
      const v = findInput(d).volume;
      return { inputVolumeMul: v, inputVolumeDb: v > 0 ? 20 * Math.log10(v) : -100 };
    },
    SetInputVolume: (d) => setVolume(findInput(d), d),
    GetInputAudioMonitorType: (d) => ({ monitorType: findInput(d).monitor ? 'OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT' : 'OBS_MONITORING_TYPE_NONE' }),

    // Éléments de scène (afficher / masquer une source)
    GetSceneItemList: (d) => ({
      sceneItems: findScene(d)
        .items.map((it, i, all) => {
          const src = store().source(it.sourceId) || {};
          return { sceneItemId: itemNumber(it), sceneItemIndex: all.length - 1 - i, sceneItemEnabled: Boolean(it.visible), sceneItemLocked: Boolean(it.locked), sourceName: src.name, sourceUuid: src.id, sourceType: 'OBS_SOURCE_TYPE_INPUT', inputKind: src.type ? inputKind(src) : null, isGroup: null };
        })
        .reverse(),
    }),
    GetSceneItemId: (d) => {
      const scene = findScene(d);
      const item = scene.items.find((it) => (store().source(it.sourceId) || {}).name === d.sourceName);
      if (!item) throw new RemoteError(CODE.ResourceNotFound, `Source introuvable dans la scène : ${d.sourceName}`);
      return { sceneItemId: itemNumber(item) };
    },
    GetSceneItemEnabled: (d) => ({ sceneItemEnabled: Boolean(findItem(d).item.visible) }),
    SetSceneItemEnabled: (d) => {
      const { scene, item } = findItem(d);
      store().updateItem(scene.id, item.id, { visible: Boolean(d.sceneItemEnabled) });
    },

    // Streaming et enregistrement
    GetStreamStatus: () => outputStatus('stream'),
    StartStream: () => startOutput('stream'),
    StopStream: () => stopOutput('stream'),
    ToggleStream: async () => {
      const on = Boolean(MS.outputs.active.stream || MS.outputs.reconnect);
      await (on ? MS.outputs.stop('stream') : MS.outputs.start('stream'));
      return { outputActive: !on };
    },
    GetRecordStatus: () => outputStatus('record'),
    StartRecord: () => startOutput('record'),
    StopRecord: async () => {
      const file = MS.outputs.active.record && MS.outputs.active.record.file;
      await stopOutput('record');
      return { outputPath: file || '' };
    },
    ToggleRecord: async () => {
      const on = Boolean(MS.outputs.active.record);
      await (on ? MS.outputs.stop('record') : MS.outputs.start('record'));
      return { outputActive: !on };
    },
    GetVirtualCamStatus: () => ({ outputActive: false }),
    GetReplayBufferStatus: () => ({ outputActive: false }),
  };

  /* Événements --------------------------------------------------------------- */

  const event = (type, intent, data) => window.api.remoteEvent({ type, intent, data });

  // État précédent, pour n'envoyer que ce qui a changé.
  let last = null;
  function snapshot() {
    const d = store().data;
    const items = new Map();
    for (const scene of d.scenes) for (const it of scene.items) items.set(it.id, { scene, it, visible: Boolean(it.visible) });
    return {
      scenes: d.scenes.map((s) => `${s.id}:${s.name}`).join('|'),
      program: d.currentSceneId,
      preview: d.previewSceneId,
      studio: Boolean(d.settings.ui.studio),
      inputs: new Map(Object.values(d.sources).map((s) => [s.id, { name: s.name, muted: Boolean(s.muted), volume: s.volume, src: s }])),
      items,
      stream: Boolean(MS.outputs.active.stream || MS.outputs.reconnect),
      record: Boolean(MS.outputs.active.record),
    };
  }

  function diff() {
    const now = snapshot();
    const prev = last;
    last = now;
    if (!prev) return;
    if (now.scenes !== prev.scenes) event('SceneListChanged', INTENT.Scenes, { scenes: sceneList() });
    if (now.program !== prev.program) {
      const s = store().scene(now.program);
      if (s) event('CurrentProgramSceneChanged', INTENT.Scenes, sceneInfo(s));
    }
    if (now.studio && now.preview !== prev.preview) {
      const s = store().scene(now.preview);
      if (s) event('CurrentPreviewSceneChanged', INTENT.Scenes, sceneInfo(s));
    }
    if (now.studio !== prev.studio) event('StudioModeStateChanged', INTENT.Ui, { studioModeEnabled: now.studio });
    for (const [id, i] of now.inputs) {
      const p = prev.inputs.get(id);
      if (!p) {
        event('InputCreated', INTENT.Inputs, { inputName: i.name, inputUuid: id, inputKind: inputKind(i.src), unversionedInputKind: inputKind(i.src), inputSettings: {}, defaultInputSettings: {} });
        continue;
      }
      if (p.name !== i.name) event('InputNameChanged', INTENT.Inputs, { inputUuid: id, oldInputName: p.name, inputName: i.name });
      if (p.muted !== i.muted) event('InputMuteStateChanged', INTENT.Inputs, { inputName: i.name, inputUuid: id, inputMuted: i.muted });
      if (p.volume !== i.volume) event('InputVolumeChanged', INTENT.Inputs, { inputName: i.name, inputUuid: id, inputVolumeMul: i.volume, inputVolumeDb: i.volume > 0 ? 20 * Math.log10(i.volume) : -100 });
    }
    for (const [id, p] of prev.inputs) if (!now.inputs.has(id)) event('InputRemoved', INTENT.Inputs, { inputName: p.name, inputUuid: id });
    for (const [id, i] of now.items) {
      const p = prev.items.get(id);
      if (p && p.visible !== i.visible) event('SceneItemEnableStateChanged', INTENT.SceneItems, { ...sceneInfo(i.scene), sceneItemId: itemNumber(i.it), sceneItemEnabled: i.visible });
    }
    const states = { true: 'OBS_WEBSOCKET_OUTPUT_STARTED', false: 'OBS_WEBSOCKET_OUTPUT_STOPPED' };
    if (now.stream !== prev.stream) event('StreamStateChanged', INTENT.Outputs, { outputActive: now.stream, outputState: states[now.stream] });
    if (now.record !== prev.record) event('RecordStateChanged', INTENT.Outputs, { outputActive: now.record, outputState: states[now.record], outputPath: null });
  }

  /* Démarrage et réglages ---------------------------------------------------- */

  const remote = {
    init() {
      window.api.onRemoteRequest(async ({ rid, type, data }) => {
        const fn = requests[type];
        if (!fn) return window.api.remoteRespond(rid, { code: CODE.UnknownRequestType, comment: `Requête non prise en charge par MyStream : ${type}` });
        try {
          const res = await fn(data || {});
          window.api.remoteRespond(rid, { code: CODE.Success, data: res || null });
        } catch (e) {
          window.api.remoteRespond(rid, { code: e.code || 702, comment: e.message });
        }
      });
      // Les changements faits à la souris ne passent pas tous par un événement : on compare l'état.
      last = snapshot();
      const check = MS.debounce(() => diff(), 30);
      for (const evt of ['scenes:changed', 'scene:switched', 'preview:switched', 'studio:changed', 'sources:changed', 'items:changed', 'mixer:changed', 'outputs:changed']) MS.on(evt, check);
      setInterval(diff, 500);
      this.apply();
      MS.on('settings:changed', () => this.apply());
    },

    async apply() {
      const cfg = store().settings.remote;
      const key = JSON.stringify(cfg);
      if (key === this.applied) return;
      this.applied = key;
      const r = await window.api.remoteConfigure({ ...cfg });
      this.error = r.ok ? null : r.error;
      if (!r.ok) MS.toast(`Télécommande (Stream Deck) indisponible : ${r.error}`, 'error', 10000);
    },

    /** Mot de passe aléatoire, comme le bouton « Générer » d'OBS. */
    newPassword() {
      const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789';
      const bytes = crypto.getRandomValues(new Uint8Array(16));
      return Array.from(bytes, (b) => chars[b % chars.length]).join('');
    },
  };

  MS.remote = remote;
})();
