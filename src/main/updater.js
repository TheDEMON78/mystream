'use strict';

// Mises à jour automatiques depuis les releases GitHub du projet.
// Windows (installateur) et Linux (AppImage) s'installent tout seuls ; les
// autres formats (portable, .deb, macOS non signé) affichent juste un lien.
// Rien n'est vérifié ni téléchargé pendant un streaming ou un enregistrement :
// tout est reporté à la fin, pour ne prendre ni bande passante ni processeur.
const { app, shell } = require('electron');
const { autoUpdater } = require('electron-updater');
let CancellationToken = null;
try {
  ({ CancellationToken } = require('builder-util-runtime'));
} catch {
  /* sans lui, un téléchargement déjà commencé ira simplement à son terme */
}

const RELEASES_URL = 'https://github.com/TheDEMON78/mystream/releases/latest';
const CHECK_EVERY = 4 * 60 * 60 * 1000;
const AFTER_BUSY = 60 * 1000; // délai après la fin du live avant de reprendre

let send = () => {};
let state = { status: 'idle' };
let isBusy = () => false;
let deferredCheck = false; // une vérification a été sautée pendant un live
let pendingDownload = null; // version à télécharger dès que possible
let download = null; // { token, version } du téléchargement en cours
let resumeTimer = null;

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

function init(sendFn, busyFn) {
  send = sendFn;
  if (busyFn) isBusy = busyFn;
  // Téléchargement géré ici, pour pouvoir le reporter ou l'interrompre pendant un live.
  autoUpdater.autoDownload = false;
  autoUpdater.autoInstallOnAppQuit = true;
  autoUpdater.logger = null;

  autoUpdater.on('checking-for-update', () => setState({ status: 'checking', error: null }));
  autoUpdater.on('update-not-available', () => setState({ status: 'latest' }));
  autoUpdater.on('update-available', (info) => {
    if (!canInstall()) return setState({ status: 'available', version: info.version });
    startDownload(info.version);
  });
  autoUpdater.on('download-progress', (p) => setState({ status: 'downloading', percent: Math.round(p.percent) }));
  autoUpdater.on('update-downloaded', (info) => {
    download = null;
    setState({ status: 'ready', version: info.version });
  });
  autoUpdater.on('error', (err) => {
    // Téléchargement interrompu exprès au début d'un live : ce n'est pas une erreur.
    if (pendingDownload && /cancel/i.test(String((err && err.message) || err))) return;
    setState({ status: 'error', error: friendly(err) });
  });

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

function startDownload(version) {
  if (isBusy()) {
    pendingDownload = version;
    setState({ status: 'waiting', version });
    return;
  }
  pendingDownload = null;
  const token = CancellationToken ? new CancellationToken() : undefined;
  download = { token, version };
  setState({ status: 'downloading', version, percent: 0 });
  autoUpdater.downloadUpdate(token).catch(() => {
    /* signalé par l'événement « error » */
  });
}

/**
 * Appelé quand un streaming ou un enregistrement démarre ou s'arrête.
 * Au début : on interrompt un téléchargement en cours. À la fin : on
 * reprend ce qui a été reporté, une minute plus tard.
 */
function setBusy(busy) {
  clearTimeout(resumeTimer);
  if (busy) {
    if (download && download.token) {
      pendingDownload = download.version;
      download.token.cancel();
      download = null;
      setState({ status: 'waiting', version: pendingDownload });
    }
    return;
  }
  resumeTimer = setTimeout(() => {
    if (isBusy()) return;
    if (pendingDownload) startDownload(pendingDownload);
    else if (deferredCheck) check();
  }, AFTER_BUSY);
}

async function check() {
  if (!app.isPackaged) {
    setState({ status: 'error', error: 'Indisponible en mode développement.' });
    return state;
  }
  if (isBusy()) {
    // Une mise à jour déjà trouvée attend la fin du live : inutile de chercher à nouveau.
    if (pendingDownload) setState({ status: 'waiting' });
    else {
      deferredCheck = true;
      setState({ status: 'deferred' });
    }
    return state;
  }
  deferredCheck = false;
  if (download || state.status === 'ready') return state;
  try {
    await autoUpdater.checkForUpdates();
  } catch (e) {
    setState({ status: 'error', error: friendly(e) });
  }
  return state;
}

function install() {
  if (isBusy()) return;
  if (state.status === 'ready') autoUpdater.quitAndInstall(false, true);
  else shell.openExternal(RELEASES_URL);
}

module.exports = { init, check, install, setBusy, getState: () => state };
