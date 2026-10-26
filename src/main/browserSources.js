'use strict';

// Sources « Navigateur » : chaque page est rendue hors écran par Chromium et
// ses images sont transmises à l'interface, qui les dessine dans la scène.
const { BrowserWindow } = require('electron');

const DEFAULT_CSS = 'body { background-color: rgba(0, 0, 0, 0); margin: 0px auto; overflow: hidden; }';
const sources = new Map();

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
  wc.on('paint', (_e, _dirty, image) => {
    const { width: w, height: h } = image.getSize();
    if (!w || !h) return;
    send('browser:frame', { id, width: w, height: h, data: image.toBitmap() });
  });
  wc.on('did-finish-load', () => {
    const css = opts.css === undefined ? DEFAULT_CSS : opts.css;
    if (css) wc.insertCSS(css).catch(() => {});
  });
  wc.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
    if (isMainFrame && code !== -3) send('browser:error', { id, error: `${desc} (${url})` });
  });

  sources.set(id, { win });
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

module.exports = { create, destroy, reload, destroyAll, DEFAULT_CSS };
