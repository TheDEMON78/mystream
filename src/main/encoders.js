'use strict';

// Détection des encodeurs vidéo matériels disponibles (NVIDIA, Intel, AMD,
// Apple) : ils soulagent fortement le processeur pendant le streaming.
const { execFile } = require('child_process');

const LABELS = {
  x264: 'Logiciel (x264)',
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

function probeOne(ffmpeg, enc) {
  return new Promise((resolve) => {
    const args = ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=black:s=320x180:r=30', '-frames:v', '3'];
    args.push('-c:v', enc, ...(enc === 'h264_qsv' ? ['-pix_fmt', 'nv12'] : []), '-f', 'null', '-');
    execFile(ffmpeg, args, { timeout: 10000, windowsHide: true }, (err) => resolve(!err));
  });
}

let cache = null;

/** Liste des encodeurs utilisables, du plus recommandé au moins recommandé. */
function detect(ffmpeg) {
  if (!cache) {
    cache = (async () => {
      const found = [];
      if (ffmpeg) {
        for (const enc of CANDIDATES[process.platform] || []) {
          if (await probeOne(ffmpeg, enc)) found.push(enc);
        }
      }
      found.push('x264');
      return found;
    })();
  }
  return cache;
}

/** Arguments FFmpeg de l'encodeur vidéo pour un débit constant (streaming). */
function videoArgs(enc, { bitrate, preset, fps }) {
  const rate = ['-b:v', `${bitrate}k`, '-maxrate', `${bitrate}k`, '-bufsize', `${bitrate * 2}k`];
  const gop = ['-g', String(fps * 2)];
  switch (enc) {
    case 'h264_nvenc':
      return ['-c:v', enc, '-preset', 'p4', '-tune', 'll', '-rc', 'cbr', ...rate, ...gop, '-pix_fmt', 'yuv420p', '-bf', '0'];
    case 'h264_qsv':
      return ['-c:v', enc, '-preset', 'veryfast', ...rate, ...gop, '-pix_fmt', 'nv12'];
    case 'h264_amf':
      return ['-c:v', enc, '-usage', 'lowlatency', '-rc', 'cbr', ...rate, ...gop, '-pix_fmt', 'yuv420p'];
    case 'h264_videotoolbox':
      return ['-c:v', enc, '-realtime', '1', ...rate, ...gop, '-pix_fmt', 'yuv420p'];
    default:
      return ['-c:v', 'libx264', '-preset', preset || 'veryfast', '-tune', 'zerolatency', ...rate, ...gop, '-sc_threshold', '0', '-pix_fmt', 'yuv420p'];
  }
}

module.exports = { detect, videoArgs, LABELS };
