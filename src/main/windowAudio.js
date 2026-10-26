'use strict';

// Son d'une fenêtre (ou du système hors MyStream) sous Windows, via le petit
// utilitaire natif window-audio.exe (native/window-audio.cpp).
const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
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
  const target = m ? `hwnd:${m[1]}` : `exclude:${process.pid}`;
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
  proc.stderr.on('data', (d) => (cap.stderr = (cap.stderr + d).slice(-500)));
  proc.on('close', (code) => {
    if (captures.get(id) !== cap) return;
    captures.delete(id);
    const error = cap.stderr.replace(/^ok\s*/m, '').trim() || `arrêt inattendu (code ${code})`;
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

function stopAll() {
  for (const id of [...captures.keys()]) stop(id);
}

module.exports = { start, stop, stopAll, available };
