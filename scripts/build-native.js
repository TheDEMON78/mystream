'use strict';

// Compile native/window-audio.cpp en local (Windows avec Visual Studio, ou
// Linux avec MinGW). En CI, le workflow de release s'en charge.
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const out = path.join(root, 'native', 'bin');
fs.mkdirSync(out, { recursive: true });
const src = path.join(root, 'native', 'window-audio.cpp');
const exe = path.join(out, 'window-audio.exe');

const cmd =
  process.platform === 'win32'
    ? `cl /nologo /O2 /EHsc /W3 /MT "${src}" ole32.lib user32.lib /Fo:"${out}\\\\" /Fe:"${exe}"`
    : `x86_64-w64-mingw32-g++ -O2 -municode -static -Wall -o "${exe}" "${src}" -lole32`;
execSync(cmd, { stdio: 'inherit' });
console.log(`Compilé : ${exe}`);
