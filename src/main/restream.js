'use strict';

// Client de l'application Restream (BloopStudio) : récupère l'adresse
// d'ingestion RTMP, l'état des relais Twitch/Kick et le chat agrégé (SSE).
const http = require('http');
const https = require('https');

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

function request(cfg, path, timeout = 5000) {
  return new Promise((resolve, reject) => {
    let url;
    try {
      const base = baseUrl(cfg);
      url = new URL(base.href.replace(/\/+$/, '') + path);
    } catch (e) {
      return reject(e);
    }
    const lib = url.protocol === 'https:' ? https : http;
    const req = lib.get(url, { ...options(cfg), timeout }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (d) => (body += d));
      res.on('end', () => {
        if (res.statusCode === 401) return reject(new Error('Mot de passe Restream incorrect.'));
        if (res.statusCode >= 400) return reject(new Error(`Restream a répondu ${res.statusCode}.`));
        try {
          resolve(JSON.parse(body));
        } catch {
          reject(new Error("Réponse inattendue : est-ce bien l'adresse de Restream ?"));
        }
      });
    });
    req.on('timeout', () => req.destroy(new Error('Restream ne répond pas.')));
    req.on('error', (e) => reject(new Error(e.code === 'ECONNREFUSED' ? 'Restream est injoignable (application lancée ?).' : e.message)));
  });
}

/** Adresse RTMP complète vers laquelle MyStream doit envoyer son flux. */
async function ingestUrl(cfg) {
  const conf = await request(cfg, '/api/config');
  if (!conf.ingestKey) throw new Error("Restream n'a pas fourni de clé d'ingestion.");
  // « localhost » peut se résoudre en IPv6 (::1) alors que le serveur RTMP
  // de Restream n'écoute qu'en IPv4 : FFmpeg resterait bloqué à la connexion.
  const host = baseUrl(cfg).hostname.replace(/^localhost$/i, '127.0.0.1');
  const h = host.includes(':') ? `[${host}]` : host;
  return { url: `rtmp://${h}:${conf.rtmpPort || 1935}/live/${conf.ingestKey}`, key: conf.ingestKey };
}

async function status(cfg) {
  const s = await request(cfg, '/api/status');
  const destinations = {};
  for (const [name, d] of Object.entries(s.destinations || {})) {
    destinations[name] = { status: d.status, failureReason: d.failureReason || null, outputBps: d.outputBps || 0 };
  }
  return { obsConnected: Boolean(s.obsConnected), incomingBps: s.incomingBps || 0, destinations };
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

module.exports = { ingestUrl, status, ChatStream };
