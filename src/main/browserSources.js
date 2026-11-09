'use strict';

// Sources « Navigateur » : chaque page est rendue hors écran par Chromium et
// ses images sont transmises à l'interface, qui les dessine dans la scène.
const path = require('path');
const { Worker } = require('worker_threads');
const { BrowserWindow } = require('electron');

const DEFAULT_CSS = 'body { background-color: rgba(0, 0, 0, 0); margin: 0px auto; overflow: hidden; }';
const sources = new Map();

function union(a, b) {
  if (!a) return { ...b };
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, width: Math.max(a.x + a.width, b.x + b.width) - x, height: Math.max(a.y + a.height, b.y + b.height) - y };
}

// Conversion des pixels dans un thread à part (voir bgraWorker.js).
let worker = null;
function convert(frame) {
  if (!worker) {
    worker = new Worker(path.join(__dirname, 'bgraWorker.js'));
    worker.on('message', (msg) => {
      const s = sources.get(msg.id);
      if (s && s.win && !s.win.isDestroyed()) s.send('browser:frame', { ...msg, data: new Uint8Array(msg.data) });
      else if (s) s.busy = false;
    });
    worker.on('error', () => (worker = null));
  }
  worker.postMessage(frame, [frame.data]);
}

/** Envoie la zone modifiée, une image à la fois : les suivantes sont fusionnées en attendant. */
function flush(id) {
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
  const bmp = part.toBitmap();
  const data = bmp.byteOffset === 0 && bmp.byteLength === bmp.buffer.byteLength ? bmp.buffer : bmp.buffer.slice(bmp.byteOffset, bmp.byteOffset + bmp.byteLength);
  convert({ id, width, height, x: r.x, y: r.y, w: r.width, h: r.height, data });
}

function ack(id) {
  const s = sources.get(id);
  if (!s) return;
  s.busy = false;
  flush(id);
}

/**
 * Source masquée ou hors de la scène : la page continue de tourner (son des
 * alertes, scripts) mais n'est plus dessinée, ni convertie, ni transmise.
 */
function setPainting(id, on) {
  const s = sources.get(id);
  if (!s || s.win.isDestroyed()) return;
  const wc = s.win.webContents;
  if (on) {
    wc.startPainting();
    s.full = true;
    wc.invalidate();
  } else {
    wc.stopPainting();
  }
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
      preload: path.join(__dirname, 'browserPreload.js'),
      // Son capturé : les fichiers audio des alertes viennent souvent d'un autre site
      // (CDN) ; sans cette exception, Chromium les rendrait muets pour la capture.
      webSecurity: !opts.audio,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  });
  const wc = win.webContents;
  // Jamais plus d'images que le canevas n'en affiche : le surplus serait perdu.
  wc.setFrameRate(Math.max(1, Math.min(Number(opts.maxFps) || 60, Number(opts.fps) || 30)));
  // Le son passe par le mixeur de MyStream (capturé dans la page), jamais directement par les haut-parleurs.
  wc.setAudioMuted(true);
  wc.setWindowOpenHandler(() => ({ action: 'deny' }));
  const state = { win, send, audio: Boolean(opts.audio), busy: false, image: null, dirty: null, full: true, scale: 1 };
  wc.on('paint', (_e, dirty, image) => {
    const { width: w, height: h } = image.getSize();
    if (!w || !h) return;
    const prev = state.image && state.image.getSize();
    if (!prev || prev.width !== w || prev.height !== h) state.full = true;
    state.scale = w / width;
    state.image = image;
    state.dirty = union(state.dirty, dirty);
    flush(id);
  });
  // Son de la page : chaque <audio>/<video> qui joue (alertes) est relié à la capture.
  if (opts.audio) {
    wc.on('dom-ready', () => {
      wc.executeJavaScript(`(() => {
        const capture = window.__myStreamCaptureMedia;
        if (!capture) return;
        window.addEventListener('play', (e) => capture(e.target), true);
        document.querySelectorAll('audio, video').forEach((el) => !el.paused && capture(el));
      })()`).catch(() => {});
    });
  }
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

/** Source « Navigateur » d'une page (d'après son webContents). */
function fromContents(wcId) {
  for (const [id, s] of sources) if (!s.win.isDestroyed() && s.win.webContents.id === wcId) return { id, state: s };
  return null;
}

/** Son de la page à envoyer au mixeur ? (demandé par le préchargement de la page) */
function audioEnabled(wcId) {
  const s = fromContents(wcId);
  return Boolean(s && s.state.audio);
}

/** Échantillons capturés dans la page : transmis à l'interface comme le son d'une fenêtre. */
function audio(wcId, pcm) {
  const s = fromContents(wcId);
  if (s && s.state.audio) s.state.send('browser:audio', { id: s.id, data: pcm });
}

function destroyAll() {
  for (const id of [...sources.keys()]) destroy(id);
}

module.exports = { create, destroy, setPainting, reload, destroyAll, ack, audioEnabled, audio, DEFAULT_CSS };
