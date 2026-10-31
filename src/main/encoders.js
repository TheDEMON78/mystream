'use strict';

// Détection des encodeurs vidéo matériels disponibles (NVIDIA, Intel, AMD,
// Apple) : ils soulagent fortement le processeur pendant le streaming.
const { execFile } = require('child_process');

const LABELS = {
  x264: 'Logiciel (x264)',
  copy: 'Sans réencodage',
  h264_nvenc: 'NVIDIA NVENC H.264',
  h264_qsv: 'Intel Quick Sync H.264',
  h264_amf: 'AMD AMF H.264',
  h264_videotoolbox: 'Apple VideoToolbox H.264',
};

const CANDIDATES = {
  win32: ['h264_nvenc', 'h264_qsv', 'h264_amf'],
  linux: ['h264_nvenc', 'h264_qsv'],
  darwin: ['h264_videotoolbox'],
};

function probeOne(ffmpeg, enc, encArgs) {
  return new Promise((resolve) => {
    const args = ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=black:s=320x180:r=30', '-frames:v', '3'];
    args.push(...encArgs, '-f', 'null', '-');
    execFile(ffmpeg, args, { timeout: 10000, windowsHide: true }, (err) => resolve(!err));
  });
}

let cache = null;
// Encodeurs dont la carte refuse les réglages « qualité » (anciennes cartes
// sans images B…) : ils gardent les réglages de base.
const basicOnly = new Set();

/** Liste des encodeurs utilisables, du plus recommandé au moins recommandé. */
function detect(ffmpeg) {
  if (!cache) {
    cache = (async () => {
      const found = [];
      if (ffmpeg) {
        for (const enc of CANDIDATES[process.platform] || []) {
          const opts = { bitrate: 2000, fps: 30 };
          if (await probeOne(ffmpeg, enc, videoArgs(enc, opts))) found.push(enc);
          else if (await probeOne(ffmpeg, enc, videoArgs(enc, { ...opts, basic: true }))) {
            basicOnly.add(enc);
            found.push(enc);
          }
        }
      }
      found.push('x264');
      return found;
    })();
  }
  return cache;
}

/** Arguments FFmpeg de l'encodeur vidéo pour un débit constant (streaming). */
function videoArgs(enc, { bitrate, preset, fps, basic }) {
  basic = basic || basicOnly.has(enc);
  const rate = ['-b:v', `${bitrate}k`, '-maxrate', `${bitrate}k`, '-bufsize', `${bitrate * 2}k`];
  const gop = ['-g', String(fps * 2)];
  switch (enc) {
    case 'h264_nvenc':
      // Qualité proche du préréglage « Qualité » d'OBS : p5, haute qualité, 2 images B.
      // Pas d'analyse anticipée ni d'AQ : elles calculent sur la carte graphique, déjà
      // occupée par le jeu, et provoquaient des saccades.
      if (basic) return ['-c:v', enc, '-preset', 'p4', '-tune', 'll', '-rc', 'cbr', ...rate, ...gop, '-pix_fmt', 'yuv420p', '-bf', '0'];
      return ['-c:v', enc, '-preset', 'p5', '-tune', 'hq', '-rc', 'cbr', ...rate, ...gop, '-pix_fmt', 'yuv420p', '-bf', '2'];
    case 'h264_qsv':
      return ['-c:v', enc, '-preset', basic ? 'veryfast' : 'medium', ...rate, ...gop, '-pix_fmt', 'nv12'];
    case 'h264_amf':
      if (basic) return ['-c:v', enc, '-usage', 'lowlatency', '-rc', 'cbr', ...rate, ...gop, '-pix_fmt', 'yuv420p'];
      return ['-c:v', enc, '-usage', 'transcoding', '-quality', 'quality', '-rc', 'cbr', ...rate, ...gop, '-pix_fmt', 'yuv420p'];
    case 'h264_videotoolbox':
      return ['-c:v', enc, '-realtime', '1', ...(basic ? [] : ['-profile:v', 'high']), ...rate, ...gop, '-pix_fmt', 'yuv420p'];
    default:
      return ['-c:v', 'libx264', '-preset', preset || 'veryfast', '-tune', 'zerolatency', ...rate, ...gop, '-sc_threshold', '0', '-pix_fmt', 'yuv420p'];
  }
}

module.exports = { detect, videoArgs, LABELS };
