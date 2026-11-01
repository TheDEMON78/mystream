'use strict';

// Réglages d'une touche MyStream dans le logiciel Stream Deck.

let sd = null;
let uuid = null;
let action = null;
let settings = {};
let lists = null;
const $ = (id) => document.getElementById(id);
const kind = () => action.split('.').pop(); // scene, mute, source, stream, record, transition

function send(event, payload) {
  sd.send(JSON.stringify({ event, context: uuid, ...(payload !== undefined ? { payload } : {}) }));
}
const toPlugin = (payload) => sd.send(JSON.stringify({ event: 'sendToPlugin', action, context: uuid, payload }));
const save = () => send('setSettings', settings);

// eslint-disable-next-line no-unused-vars
function connectElgatoStreamDeckSocket(port, piUUID, registerEvent, info, actionInfo) {
  uuid = piUUID;
  const a = JSON.parse(actionInfo);
  action = a.action;
  settings = a.payload.settings || {};
  for (const el of document.querySelectorAll('[data-for]')) el.hidden = !el.dataset.for.split(' ').includes(kind());
  $('showName').checked = settings.showName !== false;
  $('showTime').checked = settings.showTime !== false;

  sd = new WebSocket(`ws://127.0.0.1:${port}`);
  sd.onopen = () => {
    sd.send(JSON.stringify({ event: registerEvent, uuid }));
    send('getGlobalSettings');
    requestLists();
  };
  sd.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.event === 'sendToPropertyInspector') showLists(msg.payload);
    else if (msg.event === 'didReceiveGlobalSettings') {
      const g = msg.payload.settings || {};
      $('host').value = g.host || '';
      $('port').value = g.port || '';
      $('password').value = g.password || '';
    }
  };
}

function requestLists() {
  toPlugin({ cmd: 'lists', scene: settings.scene || null });
}

function fill(select, values, current, empty) {
  select.replaceChildren();
  if (empty) select.append(new Option(empty, ''));
  for (const v of values) select.append(new Option(v, v));
  // Valeur enregistrée qui n'existe plus dans MyStream : on la garde visible.
  if (current && !values.includes(current)) select.append(new Option(`${current} (introuvable)`, current));
  select.value = current || '';
}

function showLists(data) {
  const status = $('status');
  if (data.error || !data.connected) {
    status.className = 'status error';
    status.textContent = data.error || 'MyStream non connecté.';
    $('connection').open = true;
    return;
  }
  lists = data;
  status.className = 'status ok';
  status.textContent = 'Connecté à MyStream';
  fill($('scene'), data.scenes, settings.scene, 'Choisir une scène');
  fill($('source'), data.sources, settings.source, settings.scene ? 'Choisir une source' : 'Choisissez d’abord une scène');
  fill($('input'), data.inputs, settings.input, data.mic ? `Micro (${data.mic})` : 'Micro');
}

$('scene').addEventListener('change', () => {
  settings.scene = $('scene').value;
  if (kind() === 'source') settings.source = '';
  save();
  requestLists();
});
$('source').addEventListener('change', () => {
  settings.source = $('source').value;
  save();
});
$('input').addEventListener('change', () => {
  settings.input = $('input').value;
  save();
});
$('showName').addEventListener('change', () => {
  settings.showName = $('showName').checked;
  save();
});
$('showTime').addEventListener('change', () => {
  settings.showTime = $('showTime').checked;
  save();
});
$('save').addEventListener('click', () => {
  send('setGlobalSettings', { host: $('host').value.trim() || '127.0.0.1', port: Number($('port').value) || 4455, password: $('password').value });
  $('status').className = 'status';
  $('status').textContent = 'Connexion à MyStream…';
  toPlugin({ cmd: 'reconnect' });
  setTimeout(requestLists, 1500);
});

window.connectElgatoStreamDeckSocket = connectElgatoStreamDeckSocket;
