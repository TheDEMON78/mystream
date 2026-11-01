'use strict';

// Plugin Stream Deck de MyStream (BloopStudio).
//
// Le Stream Deck lance ce plugin et lui parle par une WebSocket locale. Le
// plugin se connecte de son côté à MyStream par le protocole obs-websocket v5
// (Paramètres > Stream Deck dans MyStream), envoie les commandes des touches
// et met les touches à jour à chaque changement (scène à l'antenne, micro
// coupé, live en cours…).

const ACTIONS = {
  scene: 'com.bloopstudio.mystream.scene',
  mute: 'com.bloopstudio.mystream.mute',
  source: 'com.bloopstudio.mystream.source',
  stream: 'com.bloopstudio.mystream.stream',
  record: 'com.bloopstudio.mystream.record',
  transition: 'com.bloopstudio.mystream.transition',
};
const DEFAULTS = { host: '127.0.0.1', port: 4455, password: '' };
// Catégories d'événements d'obs-websocket : scènes, entrées, sorties, éléments de scène, interface.
const SUBSCRIPTIONS = 4 | 8 | 64 | 128 | 1024;

let sd = null; // WebSocket vers le Stream Deck
let pluginUUID = null;
let globals = { ...DEFAULTS };
const keys = new Map(); // context → { action, settings }

/* Échanges avec le Stream Deck ---------------------------------------------- */

function sdSend(event, context, payload) {
  if (sd && sd.readyState === WebSocket.OPEN) sd.send(JSON.stringify({ event, context, ...(payload !== undefined ? { payload } : {}) }));
}
const setState = (ctx, state) => sdSend('setState', ctx, { state });
const setTitle = (ctx, title) => sdSend('setTitle', ctx, { title, target: 0 });
const showAlert = (ctx) => sdSend('showAlert', ctx);
const toPI = (ctx, action, payload) => sd.send(JSON.stringify({ event: 'sendToPropertyInspector', action, context: ctx, payload }));

// Point d'entrée appelé par le Stream Deck.
// eslint-disable-next-line no-unused-vars
function connectElgatoStreamDeckSocket(port, uuid, registerEvent) {
  pluginUUID = uuid;
  sd = new WebSocket(`ws://127.0.0.1:${port}`);
  sd.onopen = () => {
    sd.send(JSON.stringify({ event: registerEvent, uuid }));
    sd.send(JSON.stringify({ event: 'getGlobalSettings', context: uuid }));
  };
  sd.onmessage = (e) => onStreamDeck(JSON.parse(e.data));
}

async function onStreamDeck(msg) {
  const { event, context, action, payload = {} } = msg;
  switch (event) {
    case 'didReceiveGlobalSettings': {
      const before = JSON.stringify(globals);
      globals = { ...DEFAULTS, ...(payload.settings || {}) };
      if (JSON.stringify(globals) !== before || !obs.socket) obs.connect();
      break;
    }
    case 'willAppear':
      keys.set(context, { action, settings: payload.settings || {} });
      refreshKey(context);
      break;
    case 'willDisappear':
      keys.delete(context);
      break;
    case 'didReceiveSettings':
      if (keys.has(context)) keys.get(context).settings = payload.settings || {};
      refreshKey(context);
      break;
    case 'keyDown':
      press(context, action, payload.settings || {}).catch(() => showAlert(context));
      break;
    case 'sendToPlugin':
      onInspector(context, action, payload).catch((e) => toPI(context, action, { error: e.message }));
      break;
    default:
  }
}

/* Connexion à MyStream (obs-websocket v5) --------------------------------------- */

async function sha256Base64(text) {
  const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return btoa(String.fromCharCode(...new Uint8Array(hash)));
}

const obs = {
  socket: null,
  ready: false,
  error: 'Connexion à MyStream…',
  pending: new Map(),
  seq: 0,
  retry: null,
  state: { program: null, scenes: [], studio: false, mic: null, muted: new Map(), stream: null, record: null, items: new Map() },

  connect() {
    clearTimeout(this.retry);
    if (this.socket) {
      this.socket.onclose = null;
      this.socket.close();
    }
    this.ready = false;
    const ws = new WebSocket(`ws://${globals.host || DEFAULTS.host}:${Number(globals.port) || DEFAULTS.port}`, 'obswebsocket.json');
    this.socket = ws;
    ws.onmessage = (e) => this.onMessage(JSON.parse(e.data));
    ws.onclose = (e) => {
      if (this.socket !== ws) return;
      this.ready = false;
      this.socket = null;
      this.error = e.code === 4009 ? 'Mot de passe refusé par MyStream.' : 'MyStream injoignable : est-il ouvert, avec la télécommande activée ?';
      for (const p of this.pending.values()) p.reject(new Error(this.error));
      this.pending.clear();
      refreshAll();
      // Mot de passe faux : inutile d'insister tant qu'il n'a pas été corrigé.
      if (e.code !== 4009) this.retry = setTimeout(() => this.connect(), 3000);
    };
  },

  async onMessage({ op, d }) {
    if (op === 0) {
      // Hello : on s'identifie, avec le mot de passe si MyStream en demande un.
      const identify = { rpcVersion: 1, eventSubscriptions: SUBSCRIPTIONS };
      if (d.authentication) identify.authentication = await sha256Base64((await sha256Base64(globals.password + d.authentication.salt)) + d.authentication.challenge);
      this.socket.send(JSON.stringify({ op: 1, d: identify }));
    } else if (op === 2) {
      this.ready = true;
      this.error = null;
      await this.loadState().catch(() => {});
      refreshAll();
    } else if (op === 7) {
      const p = this.pending.get(d.requestId);
      if (!p) return;
      this.pending.delete(d.requestId);
      if (d.requestStatus.result) p.resolve(d.responseData || {});
      else p.reject(new Error(d.requestStatus.comment || `Erreur ${d.requestStatus.code}`));
    } else if (op === 5) {
      this.onEvent(d.eventType, d.eventData || {});
    }
  },

  request(requestType, requestData = {}) {
    if (!this.ready) return Promise.reject(new Error(this.error || 'MyStream non connecté.'));
    const requestId = String(++this.seq);
    return new Promise((resolve, reject) => {
      this.pending.set(requestId, { resolve, reject });
      this.socket.send(JSON.stringify({ op: 6, d: { requestType, requestId, requestData } }));
      setTimeout(() => {
        if (this.pending.delete(requestId)) reject(new Error('MyStream ne répond pas.'));
      }, 5000);
    });
  },

  async loadState() {
    const s = this.state;
    const [scenes, studio, stream, record, special] = await Promise.all([this.request('GetSceneList'), this.request('GetStudioModeEnabled'), this.request('GetStreamStatus'), this.request('GetRecordStatus'), this.request('GetSpecialInputs')]);
    s.mic = special.mic1;
    s.program = scenes.currentProgramSceneName;
    s.scenes = scenes.scenes.map((x) => x.sceneName).reverse();
    s.studio = studio.studioModeEnabled;
    s.stream = { active: stream.outputActive, since: Date.now() - (stream.outputDuration || 0) };
    s.record = { active: record.outputActive, since: Date.now() - (record.outputDuration || 0) };
    s.muted.clear();
    s.items.clear();
    for (const { action, settings } of keys.values()) {
      if (action === ACTIONS.mute) await this.loadMute(settings).catch(() => {});
      if (action === ACTIONS.source) await this.loadItem(settings).catch(() => {});
    }
  },

  /** Source audio d'une touche « Couper le son » : celle choisie, sinon le micro. */
  inputOf(settings) {
    return settings.input || this.state.mic;
  },

  async loadMute(settings) {
    const input = this.inputOf(settings);
    if (!input) return;
    const r = await this.request('GetInputMute', { inputName: input });
    this.state.muted.set(input, r.inputMuted);
  },

  async loadItem(settings) {
    if (!settings.scene || !settings.source) return;
    const { sceneItemId } = await this.request('GetSceneItemId', { sceneName: settings.scene, sourceName: settings.source });
    const r = await this.request('GetSceneItemEnabled', { sceneName: settings.scene, sceneItemId });
    this.state.items.set(`${settings.scene}\n${settings.source}`, { id: sceneItemId, enabled: r.sceneItemEnabled });
  },

  onEvent(type, d) {
    const s = this.state;
    switch (type) {
      case 'CurrentProgramSceneChanged':
        s.program = d.sceneName;
        break;
      case 'SceneListChanged':
        s.scenes = d.scenes.map((x) => x.sceneName).reverse();
        break;
      case 'StudioModeStateChanged':
        s.studio = d.studioModeEnabled;
        break;
      case 'InputMuteStateChanged':
        s.muted.set(d.inputName, d.inputMuted);
        break;
      case 'InputNameChanged':
        if (s.muted.has(d.oldInputName)) s.muted.set(d.inputName, s.muted.get(d.oldInputName));
        break;
      case 'StreamStateChanged':
        s.stream = { active: d.outputActive, since: Date.now() };
        break;
      case 'RecordStateChanged':
        s.record = { active: d.outputActive, since: Date.now() };
        break;
      case 'SceneItemEnableStateChanged':
        for (const [key, item] of s.items) if (key.startsWith(`${d.sceneName}\n`) && item.id === d.sceneItemId) item.enabled = d.sceneItemEnabled;
        break;
      default:
        return;
    }
    refreshAll();
  },
};

/* Touches --------------------------------------------------------------------- */

const pad = (n) => String(n).padStart(2, '0');
function duration(since) {
  const s = Math.floor((Date.now() - since) / 1000);
  return s >= 3600 ? `${Math.floor(s / 3600)}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}` : `${pad(Math.floor(s / 60))}:${pad(s % 60)}`;
}

function refreshKey(ctx) {
  const key = keys.get(ctx);
  if (!key) return;
  const { action, settings } = key;
  const s = obs.state;
  if (!obs.ready) {
    setState(ctx, 0);
    if (action === ACTIONS.stream || action === ACTIONS.record) setTitle(ctx, '');
    return;
  }
  switch (action) {
    case ACTIONS.scene:
      setState(ctx, settings.scene && s.program === settings.scene ? 1 : 0);
      if (settings.showName !== false) setTitle(ctx, settings.scene || '');
      break;
    case ACTIONS.mute: {
      const input = obs.inputOf(settings);
      setState(ctx, input && s.muted.get(input) ? 1 : 0);
      if (input && !s.muted.has(input)) obs.loadMute(settings).then(() => refreshKey(ctx), () => {});
      break;
    }
    case ACTIONS.source: {
      const item = s.items.get(`${settings.scene}\n${settings.source}`);
      setState(ctx, item && item.enabled ? 1 : 0);
      if (!item && settings.scene && settings.source) obs.loadItem(settings).then(() => refreshKey(ctx), () => {});
      break;
    }
    case ACTIONS.stream:
    case ACTIONS.record: {
      const out = action === ACTIONS.stream ? s.stream : s.record;
      setState(ctx, out && out.active ? 1 : 0);
      setTitle(ctx, out && out.active && settings.showTime !== false ? duration(out.since) : '');
      break;
    }
    default:
  }
}

function refreshAll() {
  for (const ctx of keys.keys()) refreshKey(ctx);
}

// Durée du live et de l'enregistrement sur les touches, mise à jour chaque seconde.
setInterval(() => {
  for (const [ctx, { action }] of keys) if (action === ACTIONS.stream || action === ACTIONS.record) refreshKey(ctx);
}, 1000);

async function press(ctx, action, settings) {
  if (!obs.ready) throw new Error(obs.error);
  switch (action) {
    case ACTIONS.scene:
      if (!settings.scene) throw new Error('Aucune scène choisie.');
      await obs.request('SetCurrentProgramScene', { sceneName: settings.scene });
      break;
    case ACTIONS.mute: {
      const input = obs.inputOf(settings);
      if (!input) throw new Error('Aucune source audio.');
      const r = await obs.request('ToggleInputMute', { inputName: input });
      obs.state.muted.set(input, r.inputMuted);
      break;
    }
    case ACTIONS.source: {
      if (!settings.scene || !settings.source) throw new Error('Aucune source choisie.');
      await obs.loadItem(settings);
      const item = obs.state.items.get(`${settings.scene}\n${settings.source}`);
      await obs.request('SetSceneItemEnabled', { sceneName: settings.scene, sceneItemId: item.id, sceneItemEnabled: !item.enabled });
      item.enabled = !item.enabled;
      break;
    }
    case ACTIONS.stream:
      await obs.request('ToggleStream');
      break;
    case ACTIONS.record:
      await obs.request('ToggleRecord');
      break;
    case ACTIONS.transition:
      if (!obs.state.studio) throw new Error('Le mode studio est désactivé.');
      await obs.request('TriggerStudioModeTransition');
      break;
    default:
  }
  refreshKey(ctx);
}

/* Fenêtre de réglages de la touche (Property Inspector) -------------------------------- */

async function onInspector(ctx, action, payload) {
  if (payload.cmd === 'reconnect') {
    obs.connect();
    return;
  }
  if (payload.cmd !== 'lists') return;
  if (!obs.ready) return toPI(ctx, action, { connected: false, error: obs.error });
  const [scenes, inputs] = await Promise.all([obs.request('GetSceneList'), obs.request('GetInputList')]);
  let sources = [];
  if (payload.scene) {
    const list = await obs.request('GetSceneItemList', { sceneName: payload.scene }).catch(() => ({ sceneItems: [] }));
    sources = list.sceneItems.map((i) => i.sourceName);
  }
  const special = await obs.request('GetSpecialInputs').catch(() => ({}));
  toPI(ctx, action, {
    connected: true,
    scenes: scenes.scenes.map((s) => s.sceneName).reverse(),
    inputs: inputs.inputs.map((i) => i.inputName),
    mic: special.mic1 || null,
    sources,
  });
}

window.connectElgatoStreamDeckSocket = connectElgatoStreamDeckSocket;
