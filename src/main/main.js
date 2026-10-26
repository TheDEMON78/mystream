'use strict';

const { app, BrowserWindow, ipcMain, desktopCapturer, dialog, shell, session, systemPreferences, Menu } = require('electron');
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn } = require('child_process');

// Les sources média doivent pouvoir jouer du son sans interaction préalable.
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

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
        send('output:ended', { id: out.id, code, error: lastLines(out.stderr) || `FFmpeg s'est arrêté (code ${code})` });
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
  if (format === 'mp4') args.push('-movflags', '+faststart');
  args.push('-y', file);
  startFfmpeg(out, args);
}

function buildStreamOutput(out, opts) {
  if (!FFMPEG) throw new Error('FFmpeg est introuvable : le streaming est indisponible.');
  const s = opts.settings.stream;
  const fps = opts.settings.video.fps;
  if (!s.server || !s.key) throw new Error('Serveur ou clé de stream manquant (Paramètres > Stream).');
  const url = `${s.server.replace(/\/+$/, '')}/${s.key}`;
  const vb = Number(s.videoBitrate) || 4500;
  const args = [
    '-hide_banner', '-loglevel', 'error', '-nostats', '-progress', 'pipe:1',
    '-i', 'pipe:0',
    '-c:v', 'libx264', '-preset', s.preset || 'veryfast', '-tune', 'zerolatency',
    '-b:v', `${vb}k`, '-maxrate', `${vb}k`, '-bufsize', `${vb * 2}k`,
    '-pix_fmt', 'yuv420p', '-r', String(fps), '-fps_mode', 'cfr',
    '-g', String(fps * 2), '-sc_threshold', '0',
    '-c:a', 'aac', '-b:a', `${Number(s.audioBitrate) || 160}k`, '-ar', '48000', '-ac', '2',
    '-f', 'flv', url,
  ];
  startFfmpeg(out, args, s.key);
}

ipcMain.handle('output:start', async (_e, opts) => {
  const out = { id: nextOutputId++, kind: opts.kind, bytes: 0, stopping: false, closed: false };
  try {
    if (opts.kind === 'record') buildRecordOutput(out, opts);
    else if (opts.kind === 'stream') buildStreamOutput(out, opts);
    else throw new Error(`Sortie inconnue : ${opts.kind}`);
  } catch (e) {
    return { ok: false, error: e.message };
  }
  outputs.set(out.id, out);
  return { ok: true, id: out.id, file: out.file || null };
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

ipcMain.handle('config:save', (_e, data) => {
  const file = configFile();
  const tmp = `${file}.tmp`;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(tmp, JSON.stringify(data, null, 2));
  fs.renameSync(tmp, file);
  return true;
});

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

ipcMain.handle('app:info', () => ({
  version: app.getVersion(),
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
  win.on('closed', () => {
    win = null;
  });
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (win) {
      if (win.isMinimized()) win.restore();
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

    createWindow();
    app.on('activate', () => {
      if (BrowserWindow.getAllWindows().length === 0) createWindow();
    });
  });

  app.on('window-all-closed', async () => {
    await Promise.all([...outputs.keys()].map(stopOutput));
    if (process.platform !== 'darwin') app.quit();
  });
}
