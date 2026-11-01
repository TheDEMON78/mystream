'use strict';

// Son d'une fenêtre (ou du système hors MyStream) sous Windows, via le petit
// utilitaire natif window-audio.exe (native/window-audio.cpp).
const path = require('path');
const fs = require('fs');
const { spawn, execFile } = require('child_process');
const { app } = require('electron');
const logger = require('./logger');

const captures = new Map();

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
    // Ne transmet que des trames complètes (2 canaux × 4 octets).
    let buf = cap.rest ? Buffer.concat([cap.rest, chunk]) : chunk;
    const usable = buf.length - (buf.length % 8);
    cap.rest = usable < buf.length ? Buffer.from(buf.subarray(usable)) : null;
    if (usable) send('winaudio:data', { id, data: new Uint8Array(buf.buffer, buf.byteOffset, usable).slice() });
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
      for (const line of lines) {
        const m = /^game (\d+) (\d+)\t([^\t]*)\t(.*)$/.exec(line.trim());
        gameWatch.current = m ? { id: `window:${m[1]}:0`, pid: Number(m[2]), exe: m[3], title: m[4] } : null;
        logger.info('Capture de jeu :', gameWatch.current ? `${gameWatch.current.exe} (${gameWatch.current.title})` : 'aucun jeu en plein écran');
        gameWatch.send('game:changed', gameWatch.current);
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
function raisePriority(pids) {
  if (!available() || !pids.length) return Promise.resolve(null);
  return new Promise((resolve) => {
    execFile(helperPath(), [`prio:${pids.join(',')}`], { windowsHide: true, timeout: 5000 }, (err, stdout) => {
      if (err) {
        logger.warn('Priorité impossible :', err.message);
        return resolve(null);
      }
      priority = String(stdout)
        .trim()
        .split(/\r?\n/)
        .map((line) => /^(\d+) cpu=(\d) gpu=(\w+)/.exec(line))
        .filter(Boolean)
        .map((m) => ({ pid: Number(m[1]), cpu: m[2] === '1', gpu: m[3] }));
      logger.info('Priorité :', priority.map((p) => `${p.pid} cpu=${p.cpu} gpu=${p.gpu}`).join(' · '));
      resolve(priority);
    });
  });
}

/** Meilleure priorité GPU obtenue : « realtime », « high » ou « none ». */
function gpuPriority() {
  if (!priority) return null;
  const levels = priority.map((p) => p.gpu);
  return levels.includes('realtime') ? 'realtime' : levels.includes('high') ? 'high' : 'none';
}

function stopAll() {
  for (const id of [...captures.keys()]) stop(id);
  gameWatch.users.clear();
  watchGames(null, false, gameWatch.send);
}

module.exports = { start, stop, stopAll, available, listApps, watchGames, currentGame: () => gameWatch.current, raisePriority, gpuPriority, priority: () => priority };
