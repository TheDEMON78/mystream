'use strict';

// Capture GPU (Windows Graphics Capture, voir native/gpu-capture.cpp) : les
// images arrivent en textures partagées, transmises telles quelles à
// l'interface (sharedTexture d'Electron), sans copie ni conversion.
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const { app, sharedTexture } = require('electron');
const logger = require('./logger');

const captures = new Map(); // id de source -> capture en cours
let target = null; // () => webContents de l'interface
let device = null; // { vendorId, deviceId } de la carte graphique de Chromium

function exePath() {
  const p = app.isPackaged ? path.join(process.resourcesPath, 'native', 'gpu-capture.exe') : path.join(__dirname, '..', '..', 'native', 'bin', 'gpu-capture.exe');
  return fs.existsSync(p) ? p : null;
}

function available() {
  return process.platform === 'win32' && Boolean(sharedTexture) && Boolean(exePath());
}

function setTarget(fn) {
  target = fn;
}

/** Carte graphique utilisée par Chromium : les textures doivent être créées sur la même. */
async function gpuDevice() {
  if (device) return device;
  try {
    const info = await app.getGPUInfo('basic');
    const list = (info && info.gpuDevice) || [];
    const d = list.find((x) => x.active) || list[0];
    device = d ? { vendorId: d.vendorId, deviceId: d.deviceId } : {};
  } catch {
    device = {};
  }
  return device;
}

/**
 * Démarre la capture. opts : { hwnd, mode: 'monitor' | 'window', fps }.
 * send('gpucap:state', { id, state, error }) signale le démarrage et les échecs.
 */
async function start(id, opts, send) {
  stop(id);
  const exe = exePath();
  if (!exe || !sharedTexture) return send('gpucap:state', { id, state: 'error', error: 'Capture GPU indisponible' });
  const d = await gpuDevice();
  const args = [String(process.pid), opts.mode === 'window' ? 'window' : 'monitor', String(opts.hwnd), String(opts.fps || 60)];
  if (d.vendorId) args.push(String(d.vendorId), String(d.deviceId || 0));
  const proc = spawn(exe, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'ignore'] });
  const cap = { proc, slots: new Map(), busy: false, frames: 0, failures: 0 };
  captures.set(id, cap);
  const free = (slot) => {
    if (!proc.killed && proc.stdin.writable) proc.stdin.write(`free ${slot}\n`);
  };
  let rest = '';
  proc.stdout.setEncoding('utf8');
  proc.stdout.on('data', (chunk) => {
    const lines = (rest + chunk).split('\n');
    rest = lines.pop();
    for (const line of lines) {
      const p = line.trim().split(' ');
      if (p[0] === 'frame') onFrame(id, cap, Number(p[1]), Number(p[2]), free);
      else if (p[0] === 'slot') {
        const handle = Buffer.alloc(8);
        handle.writeBigUInt64LE(BigInt(p[2]));
        cap.slots.set(Number(p[1]), { handle, width: Number(p[3]), height: Number(p[4]) });
      } else if (p[0] === 'error') {
        const error = line.slice(6).trim();
        logger.warn('Capture GPU :', error);
        send('gpucap:state', { id, state: 'error', error });
      } else if (p[0] === 'closed') send('gpucap:state', { id, state: 'closed' });
    }
  });
  proc.on('error', (e) => send('gpucap:state', { id, state: 'error', error: e.message }));
  proc.on('exit', (code) => {
    if (captures.get(id) !== cap) return;
    captures.delete(id);
    if (code) send('gpucap:state', { id, state: 'error', error: `Capture GPU arrêtée (code ${code})` });
  });
}

function onFrame(id, cap, slotId, timestamp, free) {
  const slot = cap.slots.get(slotId);
  const wc = target && target();
  // Une image à la fois vers l'interface : si la précédente est encore en route, celle-ci est rendue tout de suite.
  if (!slot || !wc || wc.isDestroyed() || cap.busy) return free(slotId);
  cap.busy = true;
  let imported;
  try {
    imported = sharedTexture.importSharedTexture({
      textureInfo: { pixelFormat: 'bgra', codedSize: { width: slot.width, height: slot.height }, handle: { ntHandle: slot.handle }, timestamp },
      allReferencesReleased: () => free(slotId),
    });
  } catch (e) {
    cap.busy = false;
    free(slotId);
    return failed(id, cap, e);
  }
  sharedTexture
    .sendSharedTexture({ frame: wc.mainFrame, importedSharedTexture: imported }, `cap:${id}`)
    .then(() => {
      cap.frames++;
      cap.failures = 0;
    })
    .catch((e) => failed(id, cap, e))
    .finally(() => {
      imported.release();
      cap.busy = false;
    });
}

function failed(id, cap, e) {
  if (++cap.failures < 5) return;
  logger.warn('Capture GPU : transmission impossible,', e && e.message);
  stop(id);
  const wc = target && target();
  if (wc && !wc.isDestroyed()) wc.send('gpucap:state', { id, state: 'error', error: `Transmission impossible : ${e && e.message}` });
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

module.exports = { available, setTarget, start, stop, stopAll };
