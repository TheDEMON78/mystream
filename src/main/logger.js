'use strict';

// Journal de l'application (userData/logs/mystream.log), utile pour
// comprendre un problème après coup : coupure de stream, erreur FFmpeg…
const fs = require('fs');
const path = require('path');
const { app } = require('electron');

const MAX_SIZE = 2 * 1024 * 1024;
let file = null;

function dir() {
  return path.join(app.getPath('userData'), 'logs');
}

function open() {
  if (file) return file;
  fs.mkdirSync(dir(), { recursive: true });
  file = path.join(dir(), 'mystream.log');
  try {
    // Rotation simple : l'ancien journal est gardé une fois.
    if (fs.statSync(file).size > MAX_SIZE) fs.renameSync(file, `${file}.1`);
  } catch {
    /* pas encore de journal */
  }
  return file;
}

function log(level, ...parts) {
  const line = `${new Date().toISOString()} [${level}] ${parts.map((p) => (p instanceof Error ? p.stack : typeof p === 'string' ? p : JSON.stringify(p))).join(' ')}\n`;
  try {
    fs.appendFileSync(open(), line);
  } catch {
    /* disque plein ou dossier inaccessible : on n'interrompt rien */
  }
  if (level === 'ERREUR') console.error(line.trim());
}

module.exports = {
  info: (...p) => log('INFO', ...p),
  warn: (...p) => log('ATTENTION', ...p),
  error: (...p) => log('ERREUR', ...p),
  dir,
};
