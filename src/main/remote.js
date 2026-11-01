'use strict';

// Télécommande compatible obs-websocket v5 : le Stream Deck (plugin OBS),
// Streamer.bot, Touch Portal, Bitfocus Companion… pilotent MyStream comme OBS.
// Le serveur tourne ici ; chaque requête est traitée par l'interface
// (src/renderer/js/remote.js), qui connaît les scènes et les sources.
const crypto = require('crypto');
const { WebSocketServer } = require('ws');
const msgpack = require('@msgpack/msgpack');
const logger = require('./logger');

const OP = { Hello: 0, Identify: 1, Identified: 2, Reidentify: 3, Event: 5, Request: 6, RequestResponse: 7, RequestBatch: 8, RequestBatchResponse: 9 };
const RPC_VERSION = 1;
const ALL_EVENTS = 0x7ff; // EventSubscription.All (hors événements à haute fréquence)
const STATUS = { Success: 100, UnknownRequestType: 204, RequestProcessingFailed: 702 };

let wss = null;
let current = null; // configuration en service
let invoke = null; // (type, data) => Promise<{ code, comment, data }>
const sessions = new Set();

const sha256 = (s) => crypto.createHash('sha256').update(s).digest('base64');

// Comme OBS : messages en JSON (texte) ou en MessagePack (binaire), selon le
// sous-protocole demandé par le client.
function send(ws, op, d) {
  if (ws.readyState !== ws.OPEN) return;
  if (ws.protocol === 'obswebsocket.msgpack') ws.send(msgpack.encode({ op, d }));
  else ws.send(JSON.stringify({ op, d }));
}

async function handleRequest(d) {
  const base = { requestType: d.requestType, requestId: d.requestId };
  try {
    const r = await invoke(d.requestType, d.requestData || {});
    const requestStatus = { result: r.code === STATUS.Success, code: r.code };
    if (r.comment) requestStatus.comment = r.comment;
    return { ...base, requestStatus, ...(r.data ? { responseData: r.data } : {}) };
  } catch (e) {
    return { ...base, requestStatus: { result: false, code: STATUS.RequestProcessingFailed, comment: e.message } };
  }
}

function onConnection(ws) {
  const session = { ws, identified: false, subs: ALL_EVENTS };
  const hello = { obsWebSocketVersion: '5.5.0', rpcVersion: RPC_VERSION };
  if (current.password) {
    session.challenge = crypto.randomBytes(32).toString('base64');
    session.salt = crypto.randomBytes(32).toString('base64');
    hello.authentication = { challenge: session.challenge, salt: session.salt };
  }
  sessions.add(session);
  send(ws, OP.Hello, hello);

  ws.on('message', async (raw) => {
    let msg;
    try {
      msg = ws.protocol === 'obswebsocket.msgpack' ? msgpack.decode(raw) : JSON.parse(raw.toString());
    } catch {
      return ws.close(4002, 'Message illisible.');
    }
    const d = msg.d || {};
    if (msg.op === OP.Identify) {
      if (current.password) {
        const expected = sha256(sha256(current.password + session.salt) + session.challenge);
        if (d.authentication !== expected) return ws.close(4009, 'Authentication failed.');
      }
      if (d.rpcVersion != null && d.rpcVersion > RPC_VERSION) return ws.close(4010, 'Unsupported RPC version.');
      session.identified = true;
      if (typeof d.eventSubscriptions === 'number') session.subs = d.eventSubscriptions;
      return send(ws, OP.Identified, { negotiatedRpcVersion: RPC_VERSION });
    }
    if (!session.identified) return ws.close(4007, 'Not identified.');
    if (msg.op === OP.Reidentify) {
      if (typeof d.eventSubscriptions === 'number') session.subs = d.eventSubscriptions;
      return send(ws, OP.Identified, { negotiatedRpcVersion: RPC_VERSION });
    }
    if (msg.op === OP.Request) return send(ws, OP.RequestResponse, await handleRequest(d));
    if (msg.op === OP.RequestBatch) {
      const results = [];
      for (const req of d.requests || []) {
        const r = await handleRequest(req);
        results.push(r);
        if (d.haltOnFailure && !r.requestStatus.result) break;
      }
      return send(ws, OP.RequestBatchResponse, { requestId: d.requestId, results });
    }
    return ws.close(4004, 'Unknown op code.');
  });
  ws.on('close', () => sessions.delete(session));
  ws.on('error', () => sessions.delete(session));
}

/** Événement vers les clients abonnés à sa catégorie (`intent`). */
function broadcast({ type, intent, data }) {
  for (const s of sessions) {
    if (s.identified && s.subs & intent) send(s.ws, OP.Event, { eventType: type, eventIntent: intent, ...(data ? { eventData: data } : {}) });
  }
}

function stop() {
  if (!wss) return Promise.resolve();
  const server = wss;
  wss = null;
  for (const s of sessions) s.ws.terminate();
  sessions.clear();
  return new Promise((resolve) => server.close(() => resolve()));
}

/**
 * (Re)démarre le serveur avec la configuration { enabled, port, password, lan }.
 * Renvoie { ok } ou { ok: false, error } (port déjà pris par OBS…).
 */
async function configure(cfg, handler) {
  invoke = handler;
  const key = JSON.stringify([cfg.enabled, cfg.port, cfg.lan]);
  if (wss && current && JSON.stringify([current.enabled, current.port, current.lan]) === key) {
    current = { ...cfg }; // simple changement de mot de passe : les clients connectés restent
    return { ok: true };
  }
  await stop();
  current = { ...cfg };
  if (!cfg.enabled) return { ok: true };
  return new Promise((resolve) => {
    const server = new WebSocketServer({
      host: cfg.lan ? '0.0.0.0' : '127.0.0.1',
      port: Number(cfg.port) || 4455,
      handleProtocols: (protocols) => (protocols.has('obswebsocket.json') ? 'obswebsocket.json' : protocols.has('obswebsocket.msgpack') ? 'obswebsocket.msgpack' : false),
    });
    server.once('listening', () => {
      wss = server;
      server.on('connection', onConnection);
      logger.info(`Télécommande (obs-websocket) sur le port ${cfg.port}${cfg.lan ? ', réseau local' : ''}`);
      resolve({ ok: true });
    });
    server.once('error', (e) => {
      logger.warn('Télécommande impossible :', e.message);
      resolve({ ok: false, error: e.code === 'EADDRINUSE' ? `le port ${cfg.port} est déjà utilisé (OBS est-il ouvert ?)` : e.message });
    });
  });
}

module.exports = { configure, broadcast, stop, clients: () => [...sessions].filter((s) => s.identified).length };
