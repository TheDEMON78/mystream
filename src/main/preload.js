'use strict';

const { contextBridge, ipcRenderer, sharedTexture } = require('electron');

const listen = (channel) => (fn) => ipcRenderer.on(channel, (_e, data) => fn(data));

// Images des sources Navigateur rendues sur la carte graphique : reçues sous
// forme de VideoFrame (texture GPU partagée), dessinées sans aucune copie.
let onTexture = null;
if (sharedTexture) {
  sharedTexture.setSharedTextureReceiver(async (data, id) => {
    const imported = data.importedSharedTexture;
    try {
      const frame = imported.getVideoFrame();
      if (onTexture) onTexture(id, frame);
      else frame.close();
    } finally {
      imported.release();
    }
  });
}

contextBridge.exposeInMainWorld('api', {
  loadConfig: () => ipcRenderer.invoke('config:load'),
  saveConfig: (data) => ipcRenderer.invoke('config:save', data),
  getCaptureSources: (withThumbnails = true) => ipcRenderer.invoke('capture:sources', withThumbnails),
  openFile: (opts) => ipcRenderer.invoke('dialog:openFile', opts),
  chooseDirectory: (defaultPath) => ipcRenderer.invoke('dialog:chooseDir', defaultPath),
  openPath: (p) => ipcRenderer.invoke('shell:openPath', p),
  showItemInFolder: (p) => ipcRenderer.invoke('shell:showItem', p),
  openExternal: (url) => ipcRenderer.invoke('shell:openExternal', url),
  appInfo: () => ipcRenderer.invoke('app:info'),
  stats: () => ipcRenderer.invoke('app:stats'),
  diagnostic: () => ipcRenderer.invoke('app:diagnostic'),
  encoders: () => ipcRenderer.invoke('encoders:list'),
  outputStart: (opts) => ipcRenderer.invoke('output:start', opts),
  outputChunk: (id, buffer) => ipcRenderer.send('output:chunk', id, new Uint8Array(buffer)),
  outputStop: (id) => ipcRenderer.invoke('output:stop', id),
  onOutputEnded: listen('output:ended'),
  onOutputProgress: listen('output:progress'),
  restreamTest: (cfg) => ipcRenderer.invoke('restream:test', cfg),
  restreamStatus: (cfg) => ipcRenderer.invoke('restream:status', cfg),
  restreamLive: (cfg) => ipcRenderer.invoke('restream:live', cfg),
  restreamCategories: (cfg, query) => ipcRenderer.invoke('restream:categories', cfg, query),
  restreamUpdateInfo: (cfg, info) => ipcRenderer.invoke('restream:update-info', cfg, info),
  restreamChat: (enabled, cfg) => ipcRenderer.send('restream:chat', enabled, cfg),
  twitchBadges: (channel) => ipcRenderer.invoke('twitch:badges', channel),
  kickChat: (channel) => ipcRenderer.send('kickchat:set', channel || null),
  onChatMessage: listen('chat:message'),
  onChatStatus: listen('chat:status'),
  onChatModeration: listen('chat:moderation'),
  browserCreate: (id, opts) => ipcRenderer.send('browser:create', id, opts),
  browserDestroy: (id) => ipcRenderer.send('browser:destroy', id),
  browserReload: (id) => ipcRenderer.send('browser:reload', id),
  browserAck: (id) => ipcRenderer.send('browser:ack', id),
  browserPaint: (id, on) => ipcRenderer.send('browser:paint', id, on),
  onBrowserFrame: listen('browser:frame'),
  onBrowserTexture: (fn) => (onTexture = fn),
  onBrowserError: listen('browser:error'),
  onBrowserAudio: listen('browser:audio'),
  winAudioStart: (id, captureId) => ipcRenderer.send('winaudio:start', id, captureId),
  winAudioStop: (id) => ipcRenderer.send('winaudio:stop', id),
  onWinAudioData: listen('winaudio:data'),
  onWinAudioError: listen('winaudio:error'),
  onWinAudioState: listen('winaudio:state'),
  audioApps: () => ipcRenderer.invoke('winaudio:apps'),
  remoteConfigure: (cfg) => ipcRenderer.invoke('remote:configure', cfg),
  remoteInfo: () => ipcRenderer.invoke('remote:info'),
  onRemoteRequest: listen('remote:request'),
  remoteRespond: (rid, r) => ipcRenderer.send('remote:response', rid, r),
  remoteEvent: (ev) => ipcRenderer.send('remote:event', ev),
  copyText: (text) => ipcRenderer.invoke('clipboard:write', text),
  gameWatch: (id, on) => ipcRenderer.send('game:watch', id, on),
  gameCurrent: () => ipcRenderer.invoke('game:current'),
  onGameChanged: listen('game:changed'),
  onGameForeground: listen('game:foreground'),
  updateCheck: () => ipcRenderer.invoke('update:check'),
  updateState: () => ipcRenderer.invoke('update:state'),
  updateInstall: () => ipcRenderer.send('update:install'),
  setOutputsBusy: (busy) => ipcRenderer.send('outputs:busy', busy),
  onUpdateState: listen('update:state'),
  exportConfig: (data) => ipcRenderer.invoke('config:export', data),
  importConfig: (fromBackups) => ipcRenderer.invoke('config:import', fromBackups),
  diskFree: (dir) => ipcRenderer.invoke('disk:free', dir),
  openLogs: () => ipcRenderer.invoke('logs:open'),
  log: (level, msg) => ipcRenderer.send('log', level, msg),
  setHotkeys: (map) => ipcRenderer.invoke('hotkeys:set', map),
  onHotkey: listen('hotkey'),
  setTray: (enabled) => ipcRenderer.send('tray:set', enabled),
  quit: () => ipcRenderer.send('app:quit'),
});
