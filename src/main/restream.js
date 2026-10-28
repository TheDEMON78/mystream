'use strict';

// Client de l'application Restream (BloopStudio) : récupère l'adresse
// d'ingestion RTMP, l'état des relais Twitch/Kick et le chat agrégé (SSE).
const http = require('http');
const https = require('https');
const netSocket = require('net');

function baseUrl(cfg) {
  const raw = String(cfg.url || '').trim().replace(/\/+$/, '');
  if (!raw) throw new Error("Adresse de Restream non configurée (Paramètres > Restream).");
  const url = new URL(/^https?:\/\//i.test(raw) ? raw : `http://${raw}`);
  return url;
}

function options(cfg, extraHeaders = {}) {
  const headers = { Accept: 'application/json', ...extraHeaders };
  if (cfg.password) headers.Authorization = `Basic ${Buffer.from(`mystream:${cfg.password}`).toString('base64')}`;
  return { headers, rejectUnauthorized: !cfg.allowSelfSigned };
}

function request(cfg, path, timeout = 5000, body = null) {
  return new Promise((resolve, reject) => {
    let url;
    try {
      const base = baseUrl(cfg);
      url = new URL(base.href.replace(/\/+$/, '') + path);
    } catch (e) {
      return reject(e);
    }
    const lib = url.protocol === 'https:' ? https : http;
    const payload = body ? JSON.stringify(body) : null;
    const opts = options(cfg, payload ? { 'Content-Type': 'application/json' } : {});
    const req = lib.request(url, { ...opts, method: payload ? 'POST' : 'GET', timeout }, (res) => {
      let text = '';
      res.setEncoding('utf8');
      res.on('data', (d) => (text += d));
      res.on('end', () => {
        if (res.statusCode === 401) return reject(new Error('Mot de passe Restream incorrect.'));
        let json = null;
        try {
          json = JSON.parse(text);
        } catch {
          /* réponse non JSON */
        }
        if (res.statusCode >= 400) return reject(new Error((json && json.error) || `Restream a répondu ${res.statusCode}.`));
        if (!json) return reject(new Error("Réponse inattendue : est-ce bien l'adresse de Restream ?"));
        resolve(json);
      });
    });
    req.on('timeout', () => req.destroy(new Error('Restream ne répond pas.')));
    req.on('error', (e) => reject(new Error(e.code === 'ECONNREFUSED' ? 'Restream est injoignable (application lancée ?).' : e.message)));
    req.end(payload || undefined);
  });
}

/** Adresse RTMP complète vers laquelle MyStream doit envoyer son flux. */
async function ingestUrl(cfg) {
  const conf = await request(cfg, '/api/config');
  if (!conf.ingestKey) throw new Error("Restream n'a pas fourni de clé d'ingestion.");
  // Adresse RTMP imposée : utile quand Restream est derrière un reverse proxy
  // ou quand le routeur redirige le RTMP vers un autre port public.
  const override = String(cfg.rtmpUrl || '').trim().replace(/\/+$/, '');
  if (override) {
    if (!/^rtmps?:\/\//i.test(override)) throw new Error("L'adresse RTMP doit commencer par rtmp://");
    const base = /\/live$/i.test(override) ? override : `${override}/live`;
    return { url: `${base}/${conf.ingestKey}`, key: conf.ingestKey };
  }
  // « localhost » peut se résoudre en IPv6 (::1) alors que le serveur RTMP
  // de Restream n'écoute qu'en IPv4 : FFmpeg resterait bloqué à la connexion.
  const host = baseUrl(cfg).hostname.replace(/^localhost$/i, '127.0.0.1');
  const h = host.includes(':') ? `[${host}]` : host;
  return { url: `rtmp://${h}:${conf.rtmpPort || 1935}/live/${conf.ingestKey}`, key: conf.ingestKey };
}

/**
 * Vérifie que le serveur RTMP accepte une connexion TCP avant de lancer
 * FFmpeg : un port bloqué donnerait sinon une erreur FFmpeg obscure
 * (« Error number -138 ») et une boucle de reconnexions inutiles.
 */
function checkRtmp(rtmpUrl, timeout = 5000) {
  let u;
  try {
    u = new URL(rtmpUrl);
  } catch {
    return Promise.reject(new Error(`Adresse RTMP invalide : ${rtmpUrl}`));
  }
  const host = u.hostname.replace(/^\[|\]$/g, '');
  const port = Number(u.port) || (u.protocol === 'rtmps:' ? 443 : 1935);
  const where = `${host}:${port}`;
  return new Promise((resolve, reject) => {
    const sock = netSocket.connect({ host, port });
    const fail = (msg) => {
      sock.destroy();
      reject(new Error(msg));
    };
    sock.setTimeout(timeout, () =>
      fail(
        `Le serveur RTMP ${where} ne répond pas. Le port ${port} est sans doute bloqué par un pare-feu (sur un Synology : Panneau de configuration > Sécurité > Pare-feu, autorise le port ${port} en TCP), ou l'adresse n'est pas la bonne.`
      )
    );
    sock.once('connect', () => {
      sock.destroy();
      resolve();
    });
    sock.once('error', (e) => {
      if (e.code === 'ECONNREFUSED') fail(`Le serveur RTMP ${where} refuse la connexion : Restream (ou le serveur) n'est pas lancé, ou son serveur RTMP n'a pas démarré (port ${port} déjà utilisé ? voir son journal).`);
      else if (e.code === 'ENOTFOUND' || e.code === 'EAI_AGAIN') fail(`Adresse du serveur RTMP introuvable : ${host}.`);
      else if (e.code === 'EHOSTUNREACH' || e.code === 'ENETUNREACH') fail(`Le serveur RTMP ${where} est injoignable depuis ce PC (réseau différent ou machine éteinte ?).`);
      else fail(`Connexion au serveur RTMP ${where} impossible : ${e.message}`);
    });
  });
}

async function status(cfg) {
  const s = await request(cfg, '/api/status');
  const destinations = {};
  for (const [name, d] of Object.entries(s.destinations || {})) {
    destinations[name] = { status: d.status, failureReason: d.failureReason || null, outputBps: d.outputBps || 0 };
  }
  return { obsConnected: Boolean(s.obsConnected), incomingBps: s.incomingBps || 0, destinations };
}

const PLATFORMS = ['twitch', 'kick'];

/**
 * Titre, catégorie et spectateurs de chaque plateforme, tels que Restream
 * les lit via les API officielles (comptes connectés dans Restream).
 */
async function liveInfo(cfg) {
  const out = {};
  await Promise.all(
    PLATFORMS.map(async (p) => {
      try {
        const st = await request(cfg, `/api/${p}/status`, 8000);
        out[p] = {
          connected: Boolean(st.connected),
          title: st.title || '',
          category: st.category || '',
          viewerCount: typeof st.viewerCount === 'number' ? st.viewerCount : null,
        };
      } catch (e) {
        out[p] = { connected: false, error: e.message };
      }
    })
  );
  return out;
}

/** Catégories communes aux plateformes connectées (recherche à partir de 3 lettres). */
async function categories(cfg, query) {
  const r = await request(cfg, `/api/unified/categories?query=${encodeURIComponent(query)}`, 8000);
  return Array.isArray(r.categories) ? r.categories : [];
}

/** Met à jour le titre et/ou la catégorie sur toutes les plateformes connectées. */
async function updateInfo(cfg, info) {
  const r = await request(cfg, '/api/unified/update', 10000, info);
  return r.results || {};
}

/** Flux SSE du chat agrégé, avec reconnexion automatique. */
class ChatStream {
  constructor(onMessage, onStatus) {
    this.onMessage = onMessage;
    this.onStatus = onStatus;
    this.active = false;
  }

  start(cfg) {
    this.stop();
    this.cfg = cfg;
    this.active = true;
    this.connect();
  }

  connect() {
    let url;
    try {
      url = new URL(baseUrl(this.cfg).href.replace(/\/+$/, '') + '/api/chat/stream');
    } catch (e) {
      this.onStatus('error', e.message);
      return;
    }
    const lib = url.protocol === 'https:' ? https : http;
    this.onStatus('connecting');
    const req = lib.get(url, options(this.cfg, { Accept: 'text/event-stream' }), (res) => {
      if (res.statusCode !== 200) {
        this.onStatus('error', res.statusCode === 401 ? 'Mot de passe Restream incorrect.' : `Restream a répondu ${res.statusCode}.`);
        res.resume();
        this.retry();
        return;
      }
      this.onStatus('connected');
      res.setEncoding('utf8');
      let buf = '';
      res.on('data', (chunk) => {
        buf += chunk;
        let i;
        while ((i = buf.indexOf('\n\n')) >= 0) {
          const block = buf.slice(0, i);
          buf = buf.slice(i + 2);
          const data = block
            .split('\n')
            .filter((l) => l.startsWith('data:'))
            .map((l) => l.slice(5).trimStart())
            .join('\n');
          if (!data) continue;
          try {
            this.onMessage(JSON.parse(data));
          } catch {
            /* ligne illisible ignorée */
          }
        }
      });
      res.on('end', () => this.retry());
      res.on('error', () => this.retry());
    });
    req.on('error', (e) => {
      this.onStatus('error', e.code === 'ECONNREFUSED' ? 'Restream est injoignable.' : e.message);
      this.retry();
    });
    this.req = req;
  }

  retry() {
    if (!this.active) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.active && this.connect(), 3000);
  }

  stop() {
    this.active = false;
    clearTimeout(this.timer);
    if (this.req) this.req.destroy();
    this.req = null;
  }
}

module.exports = { ingestUrl, checkRtmp, status, liveInfo, categories, updateInfo, ChatStream };
