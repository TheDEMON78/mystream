'use strict';

// Conversion des images des sources « Navigateur » (BGRA prémultiplié de
// Chromium -> RGBA de l'interface), dans un thread à part : une page plein
// écran à 30 i/s, c'est des dizaines de millions de pixels par seconde, qui
// ne doivent pas bloquer le processus principal (son, envoi à FFmpeg…).
const { parentPort } = require('worker_threads');

function toRgba(buf) {
  const px = new Uint32Array(buf);
  for (let i = 0; i < px.length; i++) {
    const p = px[i];
    const a = p >>> 24;
    if (a === 0) {
      px[i] = 0;
    } else if (a === 255) {
      px[i] = (p & 0xff00ff00) | ((p & 0xff) << 16) | ((p >>> 16) & 0xff);
    } else {
      const k = 255 / a;
      const r = Math.min(255, ((p >>> 16) & 0xff) * k);
      const g = Math.min(255, ((p >>> 8) & 0xff) * k);
      const b = Math.min(255, (p & 0xff) * k);
      px[i] = (a << 24) | (b << 16) | (g << 8) | r;
    }
  }
}

parentPort.on('message', (msg) => {
  toRgba(msg.data);
  parentPort.postMessage(msg, [msg.data]);
});
