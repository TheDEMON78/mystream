'use strict';

// Préchargé dans la page officielle de chat « popout » de Kick (fenêtre
// invisible). La page reçoit les messages en temps réel ; on observe ces
// réceptions et on transmet chaque message de chat à MyStream.
const { ipcRenderer, webFrame } = require('electron');

// Dans le monde de la page : on écoute les messages reçus par ses WebSocket.
webFrame.executeJavaScript(`(() => {
  const Native = window.WebSocket;
  function Observed(url, protocols) {
    const ws = protocols === undefined ? new Native(url) : new Native(url, protocols);
    window.postMessage({ __mystream: 'open' }, '*');
    ws.addEventListener('message', (e) => {
      if (typeof e.data === 'string' && /ChatMessageEvent|MessageDeletedEvent|UserBannedEvent|ChatroomClearEvent/.test(e.data)) window.postMessage({ __mystream: 'frame', data: e.data }, '*');
    });
    ws.addEventListener('close', () => window.postMessage({ __mystream: 'close' }, '*'));
    return ws;
  }
  Observed.prototype = Native.prototype;
  Object.assign(Observed, Native);
  window.WebSocket = Observed;
})()`);

function toMessage(raw) {
  const sender = raw.sender || {};
  const identity = sender.identity || {};
  // badges_v2 fournit les images officielles des badges ; sinon on retombe sur les types.
  const v2 = (Array.isArray(identity.badges_v2) ? identity.badges_v2 : []).map((b) => ({
    type: String(b.name || '').replace(/-/g, '_'),
    title: b.name,
    imageUrl: b.image_url || undefined,
  }));
  // Les badges « classiques » (abonné, OG, modo…) absents de badges_v2 sont ajoutés en logo.
  const known = new Set(v2.map((b) => b.type));
  const v1 = (identity.badges || [])
    .map((b) => ({ type: String(b.type || '').replace(/-/g, '_'), count: b.count }))
    .filter((b) => !known.has(b.type));
  const badges = [...v1, ...v2];
  return {
    id: raw.id,
    userId: sender.id,
    platform: 'kick',
    username: sender.username || 'inconnu',
    color: identity.color || null,
    message: String(raw.content || ''),
    badges,
    timestamp: Date.parse(raw.created_at) || Date.now(),
  };
}

window.addEventListener('message', (e) => {
  const d = e.data;
  if (!d || !d.__mystream) return;
  if (d.__mystream !== 'frame') {
    ipcRenderer.send('kickchat:status', d.__mystream === 'open' ? 'connected' : 'connecting');
    return;
  }
  // Une trame peut contenir plusieurs réponses séparées par des retours à la ligne.
  for (const line of String(d.data).split('\n')) {
    try {
      const frame = JSON.parse(line);
      const pub = frame.push && frame.push.pub && frame.push.pub.data;
      if (!pub) continue;
      const raw = typeof pub.data === 'string' ? JSON.parse(pub.data) : pub.data;
      if (!raw) continue;
      const event = String(pub.event).replace(/^App\\Events\\/, '');
      if (event === 'ChatMessageEvent' && raw.type !== 'reply-deleted') ipcRenderer.send('kickchat:message', toMessage(raw));
      // Modération : message supprimé, viewer banni ou en timeout, chat vidé.
      else if (event === 'MessageDeletedEvent') ipcRenderer.send('kickchat:moderation', { action: 'delete', id: (raw.message && raw.message.id) || raw.message_id });
      else if (event === 'UserBannedEvent' && raw.user) ipcRenderer.send('kickchat:moderation', { action: 'user', userId: raw.user.id, username: raw.user.username });
      else if (event === 'ChatroomClearEvent') ipcRenderer.send('kickchat:moderation', { action: 'clear' });
    } catch {
      /* trame d'un autre type */
    }
  }
});
