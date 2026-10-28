'use strict';

// Chat Kick sans compte ni Restream : MyStream ouvre, dans une fenêtre
// invisible, la page de chat « popout » officielle de la chaîne (celle que
// Kick propose aux streamers pour OBS) et en relaie les messages.
const path = require('path');
const { BrowserWindow, session } = require('electron');
const logger = require('./logger');

const PARTITION = 'persist:kick-chat';
let win = null;
let current = null;
let retry = null;

/** « ma_chaine », « kick.com/ma_chaine » → « ma-chaine » (format des adresses Kick). */
function slugify(channel) {
  return String(channel || '')
    .trim()
    .toLowerCase()
    .replace(/^.*kick\.com\//, '')
    .replace(/[/?#].*$/, '')
    .replace(/_/g, '-')
    .replace(/[^a-z0-9-]/g, '');
}

function stop() {
  clearTimeout(retry);
  current = null;
  if (win && !win.isDestroyed()) win.destroy();
  win = null;
}

function start(channel, send) {
  const slug = slugify(channel);
  if (!slug) return stop();
  if (current === slug && win && !win.isDestroyed()) return;
  stop();
  current = slug;

  const ses = session.fromPartition(PARTITION);
  ses.setPermissionRequestHandler((_wc, _perm, cb) => cb(false));

  win = new BrowserWindow({
    show: false,
    width: 420,
    height: 720,
    webPreferences: {
      partition: PARTITION,
      preload: path.join(__dirname, 'kickPreload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  });
  win.setMenu(null);
  const wc = win.webContents;
  wc.setAudioMuted(true);
  wc.setWindowOpenHandler(() => ({ action: 'deny' }));
  // La page reste sur le chat de la chaîne demandée.
  wc.on('will-navigate', (e, url) => {
    if (!/^https:\/\/kick\.com\/popout\//.test(url)) e.preventDefault();
  });
  wc.on('did-fail-load', (_e, code, desc, url, isMainFrame) => {
    if (!isMainFrame || code === -3) return;
    send('chat:status', { platform: 'kick', status: 'error', error: `Chat Kick injoignable (${desc}).` });
    scheduleRetry(slug, send);
  });
  wc.on('render-process-gone', () => scheduleRetry(slug, send));

  send('chat:status', { platform: 'kick', status: 'connecting' });
  logger.info(`Chat Kick : ${slug}`);
  win.loadURL(`https://kick.com/popout/${slug}/chat`).catch(() => {});
}

function scheduleRetry(slug, send) {
  clearTimeout(retry);
  retry = setTimeout(() => {
    if (current !== slug) return;
    current = null;
    start(slug, send);
  }, 10000);
}

// La page reçoit parfois le même message deux fois (plusieurs abonnements) :
// on ne garde que la première occurrence de chaque identifiant.
const seen = new Set();

function init(ipcMain, send) {
  ipcMain.on('kickchat:message', (e, msg) => {
    if (!win || e.sender !== win.webContents) return;
    if (msg.id) {
      if (seen.has(msg.id)) return;
      seen.add(msg.id);
      if (seen.size > 1000) seen.delete(seen.values().next().value);
    }
    send('chat:message', msg);
  });
  ipcMain.on('kickchat:moderation', (e, action) => {
    if (!win || e.sender !== win.webContents) return;
    send('chat:moderation', { ...action, platform: 'kick' });
  });
  ipcMain.on('kickchat:status', (e, status) => {
    if (!win || e.sender !== win.webContents) return;
    send('chat:status', { platform: 'kick', status, error: status === 'connecting' ? 'reconnexion…' : null });
  });
  ipcMain.on('kickchat:set', (_e, channel) => (channel ? start(channel, send) : stop()));
}

module.exports = { init, stop, slugify };
