'use strict';

// Sources « Navigateur » : chaque page est rendue hors écran par Chromium et
// ses images sont transmises à l'interface, qui les dessine dans la scène.
const { BrowserWindow } = require('electron');

const DEFAULT_CSS = 'body { background-color: rgba(0, 0, 0, 0); margin: 0px auto; overflow: hidden; }';
const sources = new Map();

function union(a, b) {
  if (!a) return { ...b };
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, width: Math.max(a.x + a.width, b.x + b.width) - x, height: Math.max(a.y + a.height, b.y + b.height) - y };
}

/**
 * Conversion BGRA prémultiplié (Chromium) -> RGBA (ImageData), faite ici
 * plutôt que dans l'interface pour ne pas ralentir la boucle de rendu.
 */
function toRgba(buf) {
  const px = buf.byteOffset % 4 ? new Uint32Array(Uint8Array.from(buf).buffer) : new Uint32Array(buf.buffer, buf.byteOffset, buf.length / 4);
  for (let i = 0; i < px.length; i++) {
    const p = px[i];
    const a = p >>> 24;
    if (a === 0) {
      px[i] = 0;
    } else if (a === 255) {
      px[i] = (p & 0xff00ff00) | ((p & 0xff) << 16) | ((p >>> 16) & 0xff);
    } else {
      const k = 255 / a;
      const r = Math.min(255, ((p >>> 16) & 0xff) * k);
      const g = Math.min(255, ((p >>> 8) & 0xff) * k);
      const b = Math.min(255, (p & 0xff) * k);
      px[i] = (a << 24) | (b << 16) | (g << 8) | r;
    }
  }
  return new Uint8Array(px.buffer, px.byteOffset, px.byteLength);
}

/** Envoie la zone modifiée, une image à la fois : les suivantes sont fusionnées en attendant. */
function flush(id, send) {
  const s = sources.get(id);
  if (!s || s.busy || !s.image || !s.dirty) return;
  const { width, height } = s.image.getSize();
  let r = { x: 0, y: 0, width, height };
  if (!s.full && s.scale === 1) {
    const x = Math.max(0, Math.floor(s.dirty.x));
    const y = Math.max(0, Math.floor(s.dirty.y));
    r = { x, y, width: Math.min(width - x, Math.ceil(s.dirty.width)), height: Math.min(height - y, Math.ceil(s.dirty.height)) };
  }
  s.dirty = null;
  s.full = false;
  if (r.width <= 0 || r.height <= 0) return;
  const part = r.width === width && r.height === height ? s.image : s.image.crop(r);
  s.busy = true;
  send('browser:frame', { id, width, height, x: r.x, y: r.y, w: r.width, h: r.height, data: toRgba(part.toBitmap()) });
}

function ack(id) {
  const s = sources.get(id);
  if (!s) return;
  s.busy = false;
  flush(id, s.send);
}

function destroy(id) {
  const s = sources.get(id);
  if (!s) return;
  sources.delete(id);
  if (!s.win.isDestroyed()) s.win.destroy();
}

function create(id, opts, send) {
  destroy(id);
  const width = Math.max(16, Math.min(4096, Number(opts.width) || 800));
  const height = Math.max(16, Math.min(4096, Number(opts.height) || 600));
  const win = new BrowserWindow({
    show: false,
    width,
    height,
    useContentSize: true,
    frame: false,
    transparent: true,
    backgroundColor: '#00000000',
    webPreferences: {
      offscreen: true,
      partition: 'persist:browser-sources',
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  });
  const wc = win.webContents;
  wc.setFrameRate(Math.max(1, Math.min(60, Number(opts.fps) || 30)));
  wc.setAudioMuted(!opts.audio);
  wc.setWindowOpenHandler(() => ({ action: 'deny' }));
  const state = { win, send, busy: false, image: null, dirty: null, full: true, scale: 1 };
  wc.on('paint', (_e, dirty, image) => {
    const { width: w, height: h } = image.getSize();
    if (!w || !h) return;
    const prev = state.image && state.image.getSize();
    if (!prev || prev.width !== w || prev.height !== h) state.full = true;
    state.scale = w / width;
    state.image = image;
    state.dirty = union(state.dirty, dirty);
    flush(id, send);
  });
  wc.on('did-finish-load', () => {
    const css = opts.css === undefined ? DEFAULT_CSS : opts.css;
    if (css) wc.insertCSS(css).catch(() => {});
  });
  wc.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
    if (isMainFrame && code !== -3) send('browser:error', { id, error: `${desc} (${url})` });
  });

  sources.set(id, state);
  const target = String(opts.url || '').trim();
  if (!target) return;
  if (/^(https?|file):\/\//i.test(target)) win.loadURL(target).catch(() => {});
  else win.loadFile(target).catch(() => {});
}

function reload(id) {
  const s = sources.get(id);
  if (s) s.win.webContents.reloadIgnoringCache();
}

function destroyAll() {
  for (const id of [...sources.keys()]) destroy(id);
}

module.exports = { create, destroy, reload, destroyAll, ack, DEFAULT_CSS };
