'use strict';

const { app, BrowserWindow, ipcMain, desktopCapturer, dialog, shell, session, systemPreferences, Menu, net, globalShortcut, Tray, nativeImage } = require('electron');
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn } = require('child_process');
const restream = require('./restream');
const browserSources = require('./browserSources');
const encoders = require('./encoders');
const updater = require('./updater');
const logger = require('./logger');
const windowAudio = require('./windowAudio');
const kickChat = require('./kickChat');

// Les sources média doivent pouvoir jouer du son sans interaction préalable.
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');
// Le rendu doit continuer à pleine vitesse même fenêtre réduite ou cachée.
app.commandLine.appendSwitch('disable-renderer-backgrounding');
app.commandLine.appendSwitch('disable-background-timer-throttling');
app.commandLine.appendSwitch('disable-backgrounding-occluded-windows');
app.commandLine.appendSwitch('disable-features', 'CalculateNativeWinOcclusion');

let win = null;

function getFfmpegPath() {
  try {
    const p = require('ffmpeg-static');
    if (!p) return null;
    const real = p.replace('app.asar', 'app.asar.unpacked');
    return fs.existsSync(real) ? real : null;
  } catch {
    return null;
  }
}

const FFMPEG = getFfmpegPath();
const configFile = () => path.join(app.getPath('userData'), 'config.json');

function send(channel, payload) {
  if (win && !win.isDestroyed()) win.webContents.send(channel, payload);
}

function timestampName() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}-${p(d.getMinutes())}-${p(d.getSeconds())}`;
}

/* ------------------------------------------------------------------ */
/* Sorties (enregistrement / streaming)                                */
/* ------------------------------------------------------------------ */

const outputs = new Map();
let nextOutputId = 1;

function lastLines(text, n = 6) {
  return text.trim().split(/\r?\n/).slice(-n).join('\n');
}

function startFfmpeg(out, args, secret) {
  const proc = spawn(FFMPEG, args, { windowsHide: true });
  out.proc = proc;
  out.stderr = '';
  const hide = (s) => (secret ? s.split(secret).join('****') : s);

  proc.stdin.on('error', () => {});
  proc.stderr.on('data', (d) => {
    out.stderr = (out.stderr + hide(d.toString())).slice(-4000);
  });

  let pending = '';
  let info = {};
  proc.stdout.on('data', (d) => {
    pending += d.toString();
    const lines = pending.split(/\r?\n/);
    pending = lines.pop();
    for (const line of lines) {
      const i = line.indexOf('=');
      if (i < 0) continue;
      const key = line.slice(0, i).trim();
      const value = line.slice(i + 1).trim();
      info[key] = value;
      if (key === 'progress') {
        send('output:progress', {
          id: out.id,
          bitrate: parseFloat(info.bitrate) || 0,
          fps: parseFloat(info.fps) || 0,
          speed: parseFloat(info.speed) || 0,
          dropFrames: parseInt(info.drop_frames, 10) || 0,
          totalSize: parseInt(info.total_size, 10) || 0,
        });
        info = {};
      }
    }
  });

  out.closedPromise = new Promise((resolve) => {
    proc.on('close', (code) => {
      out.closed = true;
      out.exitCode = code;
      if (!out.stopping) {
        const error = lastLines(out.stderr) || `FFmpeg s'est arrêté (code ${code})`;
        logger.error(`Sortie ${out.kind} interrompue :`, error);
        send('output:ended', { id: out.id, code, error });
      }
      resolve(code);
    });
    proc.on('error', (err) => {
      out.stderr += `\n${err.message}`;
    });
  });
}

function buildRecordOutput(out, opts) {
  const o = opts.settings.output;
  const dir = o.recordPath || app.getPath('videos');
  fs.mkdirSync(dir, { recursive: true });

  const inExt = /matroska/i.test(opts.mime) ? 'mkv' : 'webm';
  const isH264 = /avc1|h264/i.test(opts.mime);
  const format = o.format;
  const raw = !FFMPEG || format === 'webm';
  const ext = raw ? inExt : format;
  const file = path.join(dir, `MyStream ${timestampName()}.${ext}`);
  out.file = file;

  if (raw) {
    out.ws = fs.createWriteStream(file);
    out.ws.on('error', (e) => send('output:ended', { id: out.id, error: e.message }));
    return;
  }

  const args = ['-hide_banner', '-loglevel', 'error', '-nostats', '-progress', 'pipe:1', '-fflags', '+genpts', '-i', 'pipe:0'];
  if (isH264) args.push('-c:v', 'copy');
  else args.push('-c:v', 'libx264', '-preset', o.preset, '-b:v', `${o.videoBitrate}k`, '-pix_fmt', 'yuv420p');
  if (format === 'mkv') args.push('-c:a', 'copy');
  else args.push('-c:a', 'aac', '-b:a', `${o.audioBitrate}k`);
  // MP4 fragmenté : en cas de plantage ou de coupure de courant, le fichier
  // reste lisible jusqu'à la dernière seconde (un MP4 classique serait perdu).
  if (format === 'mp4') args.push('-movflags', '+frag_keyframe+empty_moov+default_base_moof', '-frag_duration', '2000000', '-flush_packets', '1');
  args.push('-y', file);
  startFfmpeg(out, args);
}

async function buildStreamOutput(out, opts) {
  if (!FFMPEG) throw new Error('FFmpeg est introuvable : le streaming est indisponible.');
  const s = opts.settings.stream;
  const fps = opts.settings.video.fps;
  let url;
  let secret;
  if (s.service === 'restream') {
    const ingest = await restream.ingestUrl(opts.settings.restream);
    url = ingest.url;
    secret = ingest.key;
  } else {
    if (!s.server || !s.key) throw new Error('Serveur ou clé de stream manquant (Paramètres > Stream).');
    url = `${s.server.replace(/\/+$/, '')}/${s.key}`;
    secret = s.key;
  }
  const available = await encoders.detect(FFMPEG);
  const enc = s.encoder === 'auto' || !available.includes(s.encoder) ? available[0] : s.encoder;
  out.encoder = enc;
  const args = [
    '-hide_banner', '-loglevel', 'error', '-nostats', '-progress', 'pipe:1',
    // Décodage matériel du flux d'entrée quand l'encodage l'est aussi : le
    // processeur n'a alors presque plus rien à faire.
    ...(enc === 'x264' ? [] : ['-hwaccel', 'auto']),
    '-i', 'pipe:0',
    ...encoders.videoArgs(enc, { bitrate: Number(s.videoBitrate) || 4500, preset: s.preset, fps }),
    '-r', String(fps), '-fps_mode', 'cfr',
    '-c:a', 'aac', '-b:a', `${Number(s.audioBitrate) || 160}k`, '-ar', '48000', '-ac', '2',
    '-f', 'flv', url,
  ];
  startFfmpeg(out, args, secret);
}

ipcMain.handle('output:start', async (_e, opts) => {
  const out = { id: nextOutputId++, kind: opts.kind, bytes: 0, stopping: false, closed: false };
  try {
    if (opts.kind === 'record') buildRecordOutput(out, opts);
    else if (opts.kind === 'stream') await buildStreamOutput(out, opts);
    else throw new Error(`Sortie inconnue : ${opts.kind}`);
  } catch (e) {
    logger.error(`Échec du démarrage ${opts.kind} :`, e.message);
    return { ok: false, error: e.message };
  }
  outputs.set(out.id, out);
  logger.info(`Démarrage ${opts.kind}`, out.file || '', out.encoder || '', opts.mime);
  return { ok: true, id: out.id, file: out.file || null, encoder: out.encoder || null };
});

ipcMain.on('output:chunk', (_e, id, data) => {
  const out = outputs.get(id);
  if (!out || out.closed) return;
  const buf = Buffer.from(data);
  out.bytes += buf.length;
  if (out.ws) out.ws.write(buf);
  else if (out.proc && out.proc.stdin.writable) out.proc.stdin.write(buf);
});

async function stopOutput(id) {
  const out = outputs.get(id);
  if (!out) return { ok: false, error: 'Sortie introuvable' };
  out.stopping = true;
  outputs.delete(id);
  logger.info(`Arrêt ${out.kind}`, out.file || '');
  if (out.ws) {
    await new Promise((resolve) => out.ws.end(resolve));
    return { ok: true, file: out.file };
  }
  if (out.proc && !out.closed) {
    out.proc.stdin.end();
    const timeout = new Promise((resolve) => setTimeout(() => resolve('timeout'), 20000));
    if ((await Promise.race([out.closedPromise, timeout])) === 'timeout') out.proc.kill('SIGKILL');
  }
  const ok = out.exitCode === 0 || out.exitCode == null;
  return { ok, file: out.file || null, error: ok ? null : lastLines(out.stderr) };
}

ipcMain.handle('output:stop', (_e, id) => stopOutput(id));

/* ------------------------------------------------------------------ */
/* Configuration, dialogues et informations                            */
/* ------------------------------------------------------------------ */

ipcMain.handle('config:load', () => {
  try {
    return JSON.parse(fs.readFileSync(configFile(), 'utf8'));
  } catch {
    return null;
  }
});

const backupDir = () => path.join(app.getPath('userData'), 'backups');

/** Une sauvegarde par jour des scènes et réglages, les 14 dernières sont gardées. */
function dailyBackup(json) {
  try {
    const dir = backupDir();
    const file = path.join(dir, `config-${new Date().toISOString().slice(0, 10)}.json`);
    if (fs.existsSync(file)) return;
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(file, json);
    const old = fs.readdirSync(dir).filter((f) => /^config-.*\.json$/.test(f)).sort();
    for (const f of old.slice(0, Math.max(0, old.length - 14))) fs.unlinkSync(path.join(dir, f));
  } catch (e) {
    logger.warn('Sauvegarde quotidienne impossible :', e.message);
  }
}

ipcMain.handle('config:save', (_e, data) => {
  const file = configFile();
  const tmp = `${file}.tmp`;
  const json = JSON.stringify(data, null, 2);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(tmp, json);
  fs.renameSync(tmp, file);
  dailyBackup(json);
  return true;
});

ipcMain.handle('config:export', async (_e, data) => {
  const res = await dialog.showSaveDialog(win, {
    defaultPath: path.join(app.getPath('documents'), `MyStream scènes ${timestampName()}.json`),
    filters: [{ name: 'Collection de scènes MyStream', extensions: ['json'] }],
  });
  if (res.canceled || !res.filePath) return null;
  fs.writeFileSync(res.filePath, JSON.stringify(data, null, 2));
  return res.filePath;
});

ipcMain.handle('config:import', async (_e, fromBackups) => {
  const res = await dialog.showOpenDialog(win, {
    defaultPath: fromBackups ? backupDir() : app.getPath('documents'),
    properties: ['openFile'],
    filters: [{ name: 'Collection de scènes MyStream', extensions: ['json'] }],
  });
  if (res.canceled || !res.filePaths[0]) return null;
  return JSON.parse(fs.readFileSync(res.filePaths[0], 'utf8'));
});

ipcMain.handle('disk:free', (_e, dir) => {
  try {
    const target = dir || app.getPath('videos');
    fs.mkdirSync(target, { recursive: true });
    const st = fs.statfsSync(target);
    return st.bavail * st.bsize;
  } catch {
    return null;
  }
});

ipcMain.handle('logs:open', () => shell.openPath(logger.dir()));
ipcMain.on('log', (_e, level, msg) => (['info', 'warn', 'error'].includes(level) ? logger[level] : logger.info)(`[interface] ${msg}`));

/* Raccourcis clavier globaux (actifs même quand un jeu a le focus) ------ */
ipcMain.handle('hotkeys:set', (_e, map) => {
  globalShortcut.unregisterAll();
  const failed = [];
  for (const [action, accel] of Object.entries(map || {})) {
    if (!accel) continue;
    try {
      if (!globalShortcut.register(accel, () => send('hotkey', action))) failed.push(accel);
    } catch {
      failed.push(accel);
    }
  }
  return failed;
});

/* Zone de notification ------------------------------------------------ */
let tray = null;
let trayEnabled = false;
ipcMain.on('tray:set', (_e, enabled) => {
  trayEnabled = Boolean(enabled);
  if (!trayEnabled && tray) {
    tray.destroy();
    tray = null;
    if (win && !win.isVisible()) win.show();
  }
});

function showTray() {
  if (tray) return;
  tray = new Tray(nativeImage.createFromPath(path.join(__dirname, '..', 'assets', 'icon.png')).resize({ width: 16, height: 16 }));
  tray.setToolTip('MyStream');
  const restore = () => {
    if (!win) return;
    win.show();
    win.focus();
    tray.destroy();
    tray = null;
  };
  tray.on('click', restore);
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: 'Afficher MyStream', click: restore },
      { label: 'Démarrer / arrêter le streaming', click: () => send('hotkey', 'toggleStream') },
      { label: "Démarrer / arrêter l'enregistrement", click: () => send('hotkey', 'toggleRecord') },
      { type: 'separator' },
      { label: 'Quitter', click: () => win && win.close() },
    ])
  );
}

ipcMain.handle('capture:sources', async (_e, withThumbnails = true) => {
  const size = withThumbnails ? { width: 320, height: 180 } : { width: 0, height: 0 };
  const sources = await desktopCapturer.getSources({ types: ['screen', 'window'], thumbnailSize: size, fetchWindowIcons: false });
  return sources.map((s) => ({
    id: s.id,
    name: s.name,
    kind: s.id.startsWith('screen') ? 'screen' : 'window',
    thumbnail: withThumbnails ? s.thumbnail.toDataURL() : null,
  }));
});

ipcMain.handle('dialog:openFile', async (_e, opts = {}) => {
  const res = await dialog.showOpenDialog(win, { properties: ['openFile'], filters: opts.filters || [] });
  return res.canceled ? null : res.filePaths[0];
});

ipcMain.handle('dialog:chooseDir', async (_e, defaultPath) => {
  const res = await dialog.showOpenDialog(win, { properties: ['openDirectory', 'createDirectory'], defaultPath });
  return res.canceled ? null : res.filePaths[0];
});

ipcMain.handle('shell:openPath', (_e, p) => shell.openPath(p || app.getPath('videos')));
ipcMain.handle('shell:showItem', (_e, p) => shell.showItemInFolder(p));
ipcMain.handle('shell:openExternal', (_e, url) => {
  if (/^https:\/\//i.test(url)) shell.openExternal(url);
});

/* ------------------------------------------------------------------ */
/* Restream, chat et sources navigateur                                */
/* ------------------------------------------------------------------ */

ipcMain.handle('encoders:list', async () => {
  const list = await encoders.detect(FFMPEG);
  return list.map((id) => ({ id, label: encoders.LABELS[id] || id }));
});

ipcMain.handle('restream:test', async (_e, cfg) => {
  try {
    const [ingest, st] = await Promise.all([restream.ingestUrl(cfg), restream.status(cfg)]);
    return { ok: true, ingest: ingest.url.replace(ingest.key, '****'), status: st };
  } catch (e) {
    return { ok: false, error: e.message };
  }
});

ipcMain.handle('restream:status', async (_e, cfg) => {
  try {
    return { ok: true, status: await restream.status(cfg) };
  } catch (e) {
    return { ok: false, error: e.message };
  }
});

const restreamChat = new restream.ChatStream(
  (msg) => send('chat:message', msg),
  (status, error) => send('chat:status', { platform: 'restream', status, error: error || null })
);
ipcMain.on('restream:chat', (_e, enabled, cfg) => {
  if (enabled) restreamChat.start(cfg);
  else restreamChat.stop();
});

// Images des badges Twitch (abonné, modo, VIP…) via l'API publique ivr.fi,
// pour le chat en lecture directe (Restream fournit déjà les siennes).
const badgeCache = new Map();
ipcMain.handle('twitch:badges', async (_e, channel) => {
  const login = String(channel || '').trim().toLowerCase().replace(/[^a-z0-9_]/g, '');
  if (badgeCache.has(login)) return badgeCache.get(login);
  const map = {};
  const load = async (url) => {
    const res = await net.fetch(url, { headers: { Accept: 'application/json' } });
    if (!res.ok) return;
    for (const set of await res.json()) {
      for (const v of set.versions || []) map[`${set.set_id}/${v.id}`] = { url: v.image_url_2x || v.image_url_1x, title: v.title };
    }
  };
  try {
    await load('https://api.ivr.fi/v2/twitch/badges/global');
    if (login) await load(`https://api.ivr.fi/v2/twitch/badges/channel?login=${login}`);
  } catch {
    /* sans réseau : les badges restent sous forme de texte */
  }
  badgeCache.set(login, map);
  return map;
});

ipcMain.on('browser:create', (_e, id, opts) => browserSources.create(id, opts, send));
ipcMain.on('browser:destroy', (_e, id) => browserSources.destroy(id));
ipcMain.on('browser:reload', (_e, id) => browserSources.reload(id));
ipcMain.on('browser:ack', (_e, id) => browserSources.ack(id));
kickChat.init(ipcMain, send);
ipcMain.on('winaudio:start', (_e, id, captureId) => windowAudio.start(id, captureId, send));
ipcMain.on('winaudio:stop', (_e, id) => windowAudio.stop(id));

ipcMain.handle('update:check', () => updater.check());
ipcMain.handle('update:state', () => updater.getState());
ipcMain.on('update:install', () => updater.install());

ipcMain.handle('app:info', () => ({
  version: app.getVersion(),
  defaultBrowserCss: browserSources.DEFAULT_CSS,
  windowAudio: windowAudio.available(),
  platform: process.platform,
  ffmpeg: Boolean(FFMPEG),
  videosPath: app.getPath('videos'),
}));

ipcMain.handle('app:stats', () => {
  const metrics = app.getAppMetrics();
  const cpu = metrics.reduce((sum, m) => sum + m.cpu.percentCPUUsage, 0) / os.cpus().length;
  const memMB = metrics.reduce((sum, m) => sum + (m.memory ? m.memory.workingSetSize : 0), 0) / 1024;
  return { cpu, memMB };
});

ipcMain.on('app:quit', () => {
  for (const id of outputs.keys()) stopOutput(id);
  app.quit();
});

/* ------------------------------------------------------------------ */
/* Fenêtre                                                             */
/* ------------------------------------------------------------------ */

function createWindow() {
  win = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 1024,
    minHeight: 680,
    backgroundColor: '#17181d',
    title: 'MyStream',
    icon: path.join(__dirname, '..', 'assets', 'icon.png'),
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  });
  win.loadFile(path.join(__dirname, '..', 'renderer', 'index.html'));
  win.on('minimize', () => {
    if (!trayEnabled || process.platform === 'darwin') return;
    showTray();
    win.hide();
  });
  win.webContents.on('console-message', (e) => {
    const level = e.level === 'error' || e.level === 3 ? 'error' : e.level === 'warning' || e.level === 2 ? 'warn' : null;
    if (level) logger[level](`[interface] ${e.message}`);
  });
  win.webContents.on('render-process-gone', (_e, d) => logger.error("Le processus de l'interface s'est arrêté :", d.reason));
  win.on('closed', () => {
    win = null;
    browserSources.destroyAll();
    windowAudio.stopAll();
    kickChat.stop();
    restreamChat.stop();
  });
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (win) {
      if (win.isMinimized()) win.restore();
      win.show();
      win.focus();
    }
  });

  app.whenReady().then(async () => {
    if (process.platform !== 'darwin') Menu.setApplicationMenu(null);

    const allowed = new Set(['media', 'display-capture', 'fullscreen']);
    session.defaultSession.setPermissionRequestHandler((_wc, permission, cb) => cb(allowed.has(permission)));
    session.defaultSession.setPermissionCheckHandler((_wc, permission) => allowed.has(permission));

    if (process.platform === 'darwin') {
      await systemPreferences.askForMediaAccess('camera').catch(() => {});
      await systemPreferences.askForMediaAccess('microphone').catch(() => {});
    }

    logger.info(`MyStream ${app.getVersion()} démarré (${process.platform}, Electron ${process.versions.electron})`);
    createWindow();
    updater.init(send);
    if (FFMPEG) encoders.detect(FFMPEG);
    app.on('activate', () => {
      if (!win) createWindow();
    });
  });

  app.on('will-quit', () => globalShortcut.unregisterAll());

  app.on('window-all-closed', async () => {
    await Promise.all([...outputs.keys()].map(stopOutput));
    if (process.platform !== 'darwin') app.quit();
  });
}
