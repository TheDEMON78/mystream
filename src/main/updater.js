'use strict';

// Mises à jour automatiques depuis les releases GitHub du projet.
// Windows (installateur) et Linux (AppImage) s'installent tout seuls ; les
// autres formats (portable, .deb, macOS non signé) affichent juste un lien.
const { app, shell } = require('electron');
const { autoUpdater } = require('electron-updater');

const RELEASES_URL = 'https://github.com/TheDEMON78/mystream/releases/latest';
const CHECK_EVERY = 4 * 60 * 60 * 1000;

let send = () => {};
let state = { status: 'idle' };

/** Formats capables de se remplacer eux-mêmes. */
function canInstall() {
  if (!app.isPackaged) return false;
  if (process.platform === 'win32') return !process.env.PORTABLE_EXECUTABLE_DIR;
  if (process.platform === 'linux') return Boolean(process.env.APPIMAGE);
  return false; // macOS : l'application n'est pas signée
}

function setState(next) {
  state = { ...state, ...next };
  send('update:state', state);
}

function init(sendFn) {
  send = sendFn;
  autoUpdater.autoDownload = canInstall();
  autoUpdater.autoInstallOnAppQuit = true;
  autoUpdater.logger = null;

  autoUpdater.on('checking-for-update', () => setState({ status: 'checking', error: null }));
  autoUpdater.on('update-not-available', () => setState({ status: 'latest' }));
  autoUpdater.on('update-available', (info) => setState({ status: canInstall() ? 'downloading' : 'available', version: info.version, percent: 0 }));
  autoUpdater.on('download-progress', (p) => setState({ status: 'downloading', percent: Math.round(p.percent) }));
  autoUpdater.on('update-downloaded', (info) => setState({ status: 'ready', version: info.version }));
  autoUpdater.on('error', (err) => setState({ status: 'error', error: friendly(err) }));

  if (!app.isPackaged) return;
  setTimeout(() => check(), 10000);
  setInterval(() => check(), CHECK_EVERY);
}

function friendly(err) {
  const msg = String((err && err.message) || err);
  if (/404|Not Found/i.test(msg)) return 'Releases introuvables (le dépôt GitHub doit être public).';
  if (/ENOTFOUND|ETIMEDOUT|ECONNRESET|net::/i.test(msg)) return 'Pas de connexion à GitHub.';
  return msg.split('\n')[0].slice(0, 200);
}

async function check() {
  if (!app.isPackaged) {
    setState({ status: 'error', error: 'Indisponible en mode développement.' });
    return state;
  }
  try {
    await autoUpdater.checkForUpdates();
  } catch (e) {
    setState({ status: 'error', error: friendly(e) });
  }
  return state;
}

function install() {
  if (state.status === 'ready') autoUpdater.quitAndInstall(false, true);
  else shell.openExternal(RELEASES_URL);
}

module.exports = { init, check, install, getState: () => state };
