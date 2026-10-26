'use strict';

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('api', {
  loadConfig: () => ipcRenderer.invoke('config:load'),
  saveConfig: (data) => ipcRenderer.invoke('config:save', data),
  getCaptureSources: (withThumbnails = true) => ipcRenderer.invoke('capture:sources', withThumbnails),
  openFile: (opts) => ipcRenderer.invoke('dialog:openFile', opts),
  chooseDirectory: (defaultPath) => ipcRenderer.invoke('dialog:chooseDir', defaultPath),
  openPath: (p) => ipcRenderer.invoke('shell:openPath', p),
  showItemInFolder: (p) => ipcRenderer.invoke('shell:showItem', p),
  appInfo: () => ipcRenderer.invoke('app:info'),
  stats: () => ipcRenderer.invoke('app:stats'),
  outputStart: (opts) => ipcRenderer.invoke('output:start', opts),
  outputChunk: (id, buffer) => ipcRenderer.send('output:chunk', id, new Uint8Array(buffer)),
  outputStop: (id) => ipcRenderer.invoke('output:stop', id),
  onOutputEnded: (fn) => ipcRenderer.on('output:ended', (_e, data) => fn(data)),
  onOutputProgress: (fn) => ipcRenderer.on('output:progress', (_e, data) => fn(data)),
  quit: () => ipcRenderer.send('app:quit'),
});
