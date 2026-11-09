'use strict';

// Son d'une fenêtre (ou du système hors MyStream) sous Windows, via le petit
// utilitaire natif window-audio.exe (native/window-audio.cpp).
const path = require('path');
const fs = require('fs');
const { spawn, execFile } = require('child_process');
const { app, screen, desktopCapturer } = require('electron');
const logger = require('./logger');

const captures = new Map();
const PCM_BATCH = 48000 * 8 * 0.02; // 20 ms de stéréo en flottants 32 bits

function helperPath() {
  const p = app.isPackaged
    ? path.join(process.resourcesPath, 'native', 'window-audio.exe')
    : path.join(__dirname, '..', '..', 'native', 'bin', 'window-audio.exe');
  return fs.existsSync(p) ? p : null;
}

function available() {
  return process.platform === 'win32' && Boolean(helperPath());
}

/**
 * Démarre la capture. `captureId` est l'identifiant Chromium de la source
 * (« window:<HWND>:0 » ou « screen:… ») : pour un écran, on capture tout le
 * son du système sauf celui de MyStream (pas d'écho du retour casque).
 */
function start(id, captureId, send) {
  stop(id);
  if (!available()) {
    send('winaudio:error', { id, error: 'Capture du son de fenêtre disponible uniquement sous Windows 10 (2004) ou plus récent.' });
    return;
  }
  const m = /^window:(\d+):/.exec(String(captureId || ''));
  const target = /^exe:/.test(captureId) ? captureId : m ? `hwnd:${m[1]}` : `exclude:${process.pid}`;
  const proc = spawn(helperPath(), [target], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const cap = { proc, rest: null, stderr: '' };
  captures.set(id, cap);

  proc.stdout.on('data', (chunk) => {
    // Paquets regroupés par 20 ms (au lieu de 10) : deux fois moins de messages
    // vers l'interface, sans latence sensible. Seules des trames complètes partent.
    const buf = cap.rest ? Buffer.concat([cap.rest, chunk]) : chunk;
    if (buf.length < PCM_BATCH) {
      cap.rest = Buffer.from(buf);
      return;
    }
    const usable = buf.length - (buf.length % 8);
    cap.rest = usable < buf.length ? Buffer.from(buf.subarray(usable)) : null;
    send('winaudio:data', { id, data: new Uint8Array(buf.buffer, buf.byteOffset, usable).slice() });
  });
  // Lignes « state … » : état de la capture (programme attendu, capturé, nom du programme).
  let pending = '';
  proc.stderr.setEncoding('utf8');
  proc.stderr.on('data', (d) => {
    const lines = (pending + d).split(/\r?\n/);
    pending = lines.pop();
    for (const line of lines) {
      const st = /^state (\w+) ?(.*)$/.exec(line);
      if (st) send('winaudio:state', { id, state: st[1], value: st[2] });
      else if (line.trim() && line.trim() !== 'ok') cap.stderr = `${cap.stderr}${line}\n`.slice(-500);
    }
  });
  proc.on('close', (code) => {
    if (captures.get(id) !== cap) return;
    captures.delete(id);
    const error = cap.stderr.trim() || `arrêt inattendu (code ${code})`;
    logger.warn(`Son de fenêtre arrêté (${target}) :`, error);
    send('winaudio:error', { id, error });
  });
  proc.on('error', (e) => logger.error('window-audio.exe :', e.message));
  logger.info(`Capture du son : ${target}`);
}

function stop(id) {
  const cap = captures.get(id);
  if (!cap) return;
  captures.delete(id);
  try {
    cap.proc.stdin.end();
    cap.proc.kill();
  } catch {
    /* déjà arrêté */
  }
}

/* Jeu en plein écran au premier plan (source « Capture de jeu ») --------- */

const gameWatch = { proc: null, users: new Set(), current: null, send: null };

/** Démarre la surveillance tant qu'au moins une source « Capture de jeu » existe. */
function watchGames(id, on, send) {
  gameWatch.send = send;
  if (on) gameWatch.users.add(id);
  else gameWatch.users.delete(id);
  if (gameWatch.users.size && !gameWatch.proc && available()) {
    const proc = spawn(helperPath(), [`games:${process.pid}`], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    gameWatch.proc = proc;
    let rest = '';
    proc.stdout.setEncoding('utf8');
    proc.stdout.on('data', (chunk) => {
      const lines = (rest + chunk).split('\n');
      rest = lines.pop();
      for (const raw of lines) {
        const line = raw.trim();
        const fg = /^fg ([01])$/.exec(line);
        if (fg) {
          if (gameWatch.current) gameWatch.send('game:foreground', fg[1] === '1');
          continue;
        }
        const m = /^game (\d+) (\d+)\t([^\t]*)\t([^\t]*)\t(-?\d+),(-?\d+),(-?\d+),(-?\d+)$/.exec(line);
        const game = m ? { id: `window:${m[1]}:0`, pid: Number(m[2]), exe: m[3], title: m[4], screenId: null } : null;
        gameWatch.current = game;
        logger.info('Capture de jeu :', game ? `${game.exe} (${game.title})` : 'aucun jeu en plein écran');
        if (!game) {
          gameWatch.send('game:changed', null);
          continue;
        }
        screenOf(m.slice(5, 9).map(Number)).then((screenId) => {
          if (gameWatch.current !== game) return;
          game.screenId = screenId;
          gameWatch.send('game:changed', game);
        });
      }
    });
    proc.on('close', () => {
      if (gameWatch.proc !== proc) return;
      gameWatch.proc = null;
      gameWatch.current = null;
      gameWatch.send('game:changed', null);
    });
    proc.on('error', (e) => logger.error('Surveillance des jeux :', e.message));
  } else if (!gameWatch.users.size && gameWatch.proc) {
    const proc = gameWatch.proc;
    gameWatch.proc = null;
    gameWatch.current = null;
    try {
      proc.stdin.end();
      proc.kill();
    } catch {
      /* déjà arrêté */
    }
  }
}

/**
 * Programmes ouverts : ceux qui utilisent le son (même sans fenêtre, comme
 * Discord dans la zone de notification), puis ceux qui ont une fenêtre.
 * [{ exe, kind: 'audio' | 'window', title }]
 */
function listApps() {
  if (!available()) return Promise.resolve([]);
  return new Promise((resolve) => {
    execFile(helperPath(), ['apps'], { windowsHide: true, timeout: 8000 }, (err, stdout) => {
      if (err) return resolve([]);
      const apps = String(stdout)
        .split(/\r?\n/)
        .map((line) => line.split('\t'))
        .filter(([kind, exe]) => (kind === 'audio' || kind === 'window') && exe)
        .map(([kind, exe, title]) => ({ exe, kind, title: title || '' }));
      const order = { audio: 0, window: 1 };
      resolve(apps.sort((a, b) => order[a.kind] - order[b.kind] || a.exe.localeCompare(b.exe, 'fr', { sensitivity: 'base' })));
    });
  });
}

/* Priorité face au jeu --------------------------------------------------- */

let priority = null;

/**
 * Donne aux processus de MyStream la priorité sur le jeu, pour le processeur
 * et la carte graphique (comme OBS). La priorité GPU maximale demande de
 * lancer MyStream en administrateur ; sinon on obtient la priorité « haute »
 * quand Windows l'accorde.
 */
function raisePriority(targets) {
  // targets : [{ pid, type }] (type : Browser, Tab, GPU, FFmpeg…)
  const pids = targets.map((t) => t.pid);
  if (!available() || !pids.length) return Promise.resolve(null);
  return new Promise((resolve) => {
    execFile(helperPath(), [`prio:${pids.join(',')}`], { windowsHide: true, timeout: 5000 }, (err, stdout) => {
      if (err) {
        logger.warn('Priorité impossible :', err.message);
        return resolve(null);
      }
      const at = Math.round(process.uptime());
      const results = String(stdout)
        .trim()
        .split(/\r?\n/)
        .map((line) => /^(\d+) cpu=(\d) gpu=(\w+)(?: ms=(\d+))?/.exec(line))
        .filter(Boolean)
        .map((m) => ({ pid: Number(m[1]), type: (targets.find((t) => t.pid === Number(m[1])) || {}).type, cpu: m[2] === '1', gpu: m[3], ms: Number(m[4] || 0), at }));
      // On garde le dernier résultat de chaque processus (diagnostic).
      const byPid = new Map((priority || []).map((p) => [p.pid, p]));
      for (const r of results) byPid.set(r.pid, r);
      priority = [...byPid.values()];
      logger.info('Priorité :', results.map((p) => `${p.type} ${p.pid} cpu=${p.cpu} gpu=${p.gpu} (${p.ms} ms)`).join(' · '));
      resolve(results);
    });
  });
}

/** Meilleure priorité GPU obtenue : « high » ou « none ». */
function gpuPriority() {
  if (!priority) return null;
  const levels = priority.map((p) => p.gpu);
  return levels.includes('high') ? 'high' : 'none';
}

/**
 * Écran (source de capture Chromium) qui affiche le jeu, d'après son rectangle
 * en pixels physiques. Capturer l'écran passe par la duplication d'écran de la
 * carte graphique : bien plus fluide et léger que la capture de la fenêtre
 * d'un jeu, qui fait aussi ramer le jeu.
 */
async function screenOf([left, top, right, bottom]) {
  try {
    const dip = screen.screenToDipRect(null, { x: left, y: top, width: right - left, height: bottom - top });
    const display = screen.getAllDisplays().find((d) => Math.abs(d.bounds.x - dip.x) <= 2 && Math.abs(d.bounds.y - dip.y) <= 2 && Math.abs(d.bounds.width - dip.width) <= 2);
    if (!display) return null;
    const sources = await desktopCapturer.getSources({ types: ['screen'], thumbnailSize: { width: 0, height: 0 } });
    const src = sources.find((x) => x.display_id === String(display.id));
    return src ? src.id : null;
  } catch (e) {
    logger.warn('Écran du jeu introuvable :', e.message);
    return null;
  }
}

/**
 * Horloge de rendu précise (voir renderClock dans window-audio.cpp) :
 * `onTick` est appelé à chaque image, à l'instant exact.
 */
let clock = null;
function renderClock(fps, onTick) {
  if (clock) {
    clock.kill();
    clock = null;
  }
  if (!fps || !available()) return false;
  const proc = spawn(helperPath(), [`clock:${fps}`], { windowsHide: true, stdio: ['ignore', 'pipe', 'ignore'] });
  clock = proc;
  proc.stdout.on('data', () => onTick());
  proc.on('error', () => {});
  proc.on('exit', () => {
    if (clock === proc) clock = null;
  });
  return true;
}

/**
 * Charge du processeur (totale et par processus) et de la carte graphique
 * pendant 1 s, pour le diagnostic. targets : [{ pid, type }].
 */
function usage(targets) {
  return new Promise((resolve) => {
    if (!available() || !targets.length) return resolve(null);
    execFile(helperPath(), [`usage:${targets.map((t) => t.pid).join(',')}`], { windowsHide: true, timeout: 5000 }, (err, out) => {
      if (err && !out) return resolve(null);
      const byPid = new Map(targets.map((t) => [t.pid, t.type]));
      const res = { cpu: null, gpu: {}, processes: [] };
      for (const line of String(out).split(/\r?\n/)) {
        let m = /^system cpu=([\d.]+)/.exec(line);
        if (m) res.cpu = Number(m[1]);
        m = /^(\d+) cpu=(-?[\d.]+)/.exec(line);
        if (m) res.processes.push({ type: byPid.get(Number(m[1])) || m[1], cpu: Number(m[2]) });
        m = /^gpu (\S+) ([\d.]+)(.*)$/.exec(line);
        if (m) {
          const per = {};
          for (const [, pid, v] of m[3].matchAll(/(\d+)=([\d.]+)/g)) per[byPid.get(Number(pid)) || pid] = Number(v);
          res.gpu[m[1]] = { total: Number(m[2]), ...per };
        }
      }
      resolve(res);
    });
  });
}

function stopAll() {
  for (const id of [...captures.keys()]) stop(id);
  gameWatch.users.clear();
  watchGames(null, false, gameWatch.send);
  renderClock(0);
}

module.exports = { start, stop, stopAll, available, renderClock, usage, listApps, watchGames, currentGame: () => gameWatch.current, raisePriority, gpuPriority, priority: () => priority };
