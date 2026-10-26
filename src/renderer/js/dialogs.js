'use strict';

(function () {
  const MS = window.MS;
  const el = MS.el;

  /* Fenêtre modale générique ------------------------------------------- */

  MS.modal = function ({ title, content, buttons, width = 520, onOpen }) {
    return new Promise((resolve) => {
      const btns = buttons || [
        { label: 'Annuler', value: false },
        { label: 'OK', value: true, primary: true },
      ];
      let done = false;
      const close = (value) => {
        if (done) return;
        done = true;
        backdrop.remove();
        document.removeEventListener('keydown', onKey, true);
        resolve(value);
      };
      const onKey = (e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          close(false);
        } else if (e.key === 'Enter' && e.target.tagName !== 'TEXTAREA' && e.target.tagName !== 'BUTTON') {
          const primary = btns.find((b) => b.primary);
          if (primary) {
            e.preventDefault();
            close(primary.value);
          }
        }
      };
      const backdrop = el(
        'div',
        { class: 'modal-backdrop' },
        el(
          'div',
          { class: 'modal', style: { width: `${width}px` } },
          el('div', { class: 'modal-title', text: title }),
          el('div', { class: 'modal-body' }, content),
          el(
            'div',
            { class: 'modal-buttons' },
            btns.map((b) =>
              el('button', {
                class: `${b.primary ? 'btn primary' : 'btn'} ${b.left ? 'left' : ''}`,
                text: b.label,
                onclick: () => (b.onClick ? b.onClick() : close(b.value)),
              })
            )
          )
        )
      );
      document.getElementById('modal-root').append(backdrop);
      document.addEventListener('keydown', onKey, true);
      const first = backdrop.querySelector('input, textarea, select');
      if (first) first.focus();
      if (onOpen) onOpen(close);
    });
  };

  MS.prompt = async function (title, label, value = '') {
    const input = el('input', { type: 'text', class: 'input', value });
    const ok = await MS.modal({ title, content: el('label', { class: 'field' }, el('span', { text: label }), input), width: 420 });
    const v = input.value.trim();
    return ok && v ? v : null;
  };

  MS.confirm = (title, message, okLabel = 'OK') =>
    MS.modal({
      title,
      content: el('p', { text: message }),
      width: 420,
      buttons: [
        { label: 'Annuler', value: false },
        { label: okLabel, value: true, primary: true },
      ],
    });

  /* Formulaires --------------------------------------------------------- */

  /**
   * Construit un formulaire à partir d'une liste de champs.
   * `onChange` est appelé à chaque modification avec les valeurs courantes.
   */
  MS.buildForm = function (fields, values, onChange) {
    const root = el('div', { class: 'form' });
    const getters = {};
    const conditional = [];
    // Champs affichés selon les autres valeurs (ex. serveur masqué avec Restream).
    const refresh = () => {
      const vals = collect();
      for (const [node, test] of conditional) node.hidden = !test(vals);
    };
    const changed = () => {
      refresh();
      if (onChange) onChange(collect());
    };
    const collect = () => {
      const out = {};
      for (const [k, g] of Object.entries(getters)) out[k] = g();
      return out;
    };

    for (const f of fields) {
      const before = root.childElementCount;
      appendField(f);
      if (f.showIf && root.childElementCount > before) conditional.push([root.lastElementChild, f.showIf]);
    }
    refresh();
    return { element: root, values: collect };

    function appendField(f) {
      if (f.type === 'info') {
        root.append(el('p', { class: 'hint', text: f.text }));
        return;
      }
      if (f.type === 'button') {
        root.append(el('div', { class: 'field' }, el('button', { class: 'btn', text: f.label, onclick: (e) => f.onClick(collect(), e.currentTarget) })));
        return;
      }
      if (f.type === 'section') {
        root.append(el('div', { class: 'form-section', text: f.label }));
        return;
      }
      let input;
      const v = values[f.key];
      switch (f.type) {
        case 'textarea':
          input = el('textarea', { class: 'input', rows: 4, value: v ?? '', oninput: changed });
          getters[f.key] = () => input.value;
          break;
        case 'number':
          input = el('input', { class: 'input', type: 'number', min: f.min, max: f.max, step: f.step || 1, value: v ?? 0, oninput: changed });
          getters[f.key] = () => Number(input.value);
          break;
        case 'range': {
          const out = el('span', { class: 'range-value', text: v });
          const r = el('input', {
            type: 'range',
            min: f.min,
            max: f.max,
            value: v ?? 0,
            oninput: () => {
              out.textContent = r.value;
              changed();
            },
          });
          input = el('div', { class: 'range' }, r, out);
          getters[f.key] = () => Number(r.value);
          break;
        }
        case 'color':
          input = el('input', { class: 'color', type: 'color', value: v || '#ffffff', oninput: changed });
          getters[f.key] = () => input.value;
          break;
        case 'checkbox': {
          const cb = el('input', { type: 'checkbox', checked: Boolean(v), onchange: changed });
          root.append(el('label', { class: 'field checkbox' }, cb, el('span', { text: f.label })));
          getters[f.key] = () => cb.checked;
          return;
        }
        case 'select': {
          const opts = f.options || [];
          const known = opts.some((o) => String(o.value) === String(v));
          input = el(
            'select',
            { class: 'input', onchange: changed },
            opts.map((o) => el('option', { value: o.value, text: o.label, selected: String(o.value) === String(v) })),
            !known && v ? el('option', { value: v, text: `(indisponible) ${v}`, selected: true }) : null
          );
          getters[f.key] = () => input.value;
          break;
        }
        case 'file':
        case 'directory': {
          const text = el('input', { class: 'input', type: 'text', value: v || '', placeholder: f.placeholder || '', oninput: changed });
          const browse = el('button', {
            class: 'btn',
            text: 'Parcourir…',
            onclick: async () => {
              const p = f.type === 'file' ? await window.api.openFile({ filters: f.filters }) : await window.api.chooseDirectory(text.value || undefined);
              if (p) {
                text.value = p;
                changed();
              }
            },
          });
          input = el('div', { class: 'file-row' }, text, browse);
          getters[f.key] = () => text.value;
          break;
        }
        case 'hotkey': {
          // Capture de la combinaison pressée ; Retour arrière ou Suppr pour effacer.
          const hk = el('input', { class: 'input hotkey', type: 'text', readonly: true, value: v || '', placeholder: 'Cliquer puis appuyer sur les touches' });
          hk.addEventListener('keydown', (e) => {
            e.preventDefault();
            e.stopPropagation();
            if (e.key === 'Backspace' || e.key === 'Delete') hk.value = '';
            else {
              const acc = MS.keyToAccelerator(e);
              if (acc) hk.value = acc;
            }
            changed();
          });
          input = hk;
          getters[f.key] = () => hk.value;
          break;
        }
        case 'password': {
          const pw = el('input', { class: 'input', type: 'password', value: v || '', oninput: changed });
          const toggle = el('button', {
            class: 'btn',
            text: 'Afficher',
            onclick: () => {
              pw.type = pw.type === 'password' ? 'text' : 'password';
              toggle.textContent = pw.type === 'password' ? 'Afficher' : 'Masquer';
            },
          });
          input = el('div', { class: 'file-row' }, pw, toggle);
          getters[f.key] = () => pw.value;
          break;
        }
        case 'capture': {
          const picker = MS.capturePicker(values, changed);
          input = picker.element;
          Object.assign(getters, picker.getters);
          break;
        }
        default:
          input = el('input', { class: 'input', type: 'text', value: v ?? '', placeholder: f.placeholder || '', oninput: changed });
          getters[f.key] = () => input.value;
      }
      root.append(el('label', { class: `field ${f.type === 'capture' ? 'wide' : ''}` }, el('span', { text: f.label }), input));
    }
  };

  /* Sélecteur d'écran / fenêtre ----------------------------------------- */

  MS.capturePicker = function (values, onChange) {
    const state = { captureId: values.captureId, captureName: values.captureName, captureKind: values.captureKind };
    const grid = el('div', { class: 'capture-grid' }, el('p', { class: 'hint', text: 'Recherche des écrans et fenêtres…' }));
    const refresh = async () => {
      let list = [];
      try {
        list = await window.api.getCaptureSources(true);
      } catch (e) {
        grid.replaceChildren(el('p', { class: 'hint', text: `Impossible de lister les sources : ${e.message}` }));
        return;
      }
      if (!state.captureId && list[0]) Object.assign(state, { captureId: list[0].id, captureName: list[0].name, captureKind: list[0].kind });
      grid.replaceChildren(
        ...list.map((s) => {
          const card = el(
            'button',
            {
              class: `capture-card ${s.id === state.captureId ? 'selected' : ''}`,
              title: s.name,
              onclick: () => {
                Object.assign(state, { captureId: s.id, captureName: s.name, captureKind: s.kind });
                grid.querySelectorAll('.capture-card').forEach((c) => c.classList.remove('selected'));
                card.classList.add('selected');
                onChange();
              },
            },
            el('img', { src: s.thumbnail, alt: '' }),
            el('span', { text: `${s.kind === 'screen' ? 'Écran' : 'Fenêtre'} · ${s.name}` })
          );
          return card;
        })
      );
      onChange();
    };
    refresh();
    const element = el('div', {}, grid, el('button', { class: 'btn small', text: 'Actualiser', onclick: refresh }));
    return {
      element,
      getters: { captureId: () => state.captureId, captureName: () => state.captureName, captureKind: () => state.captureKind },
    };
  };

  /* Propriétés d'une source --------------------------------------------- */

  MS.openProperties = async function (sourceId) {
    const src = MS.store.source(sourceId);
    if (!src) return false;
    const type = MS.SourceTypes[src.type];
    const backup = JSON.parse(JSON.stringify(src.settings));
    const fields = await type.fields(src.settings);
    const form = MS.buildForm(fields, src.settings, (vals) => {
      if (!type.live) return;
      Object.assign(src.settings, vals);
      MS.runtimes.refresh(src);
    });
    const nameInput = el('input', { class: 'input', type: 'text', value: src.name });
    const content = el(
      'div',
      {},
      src.global ? null : el('label', { class: 'field' }, el('span', { text: 'Nom' }), nameInput),
      form.element,
      (type.actions || []).map((a) => el('button', { class: 'btn', text: a.label, onclick: () => a.run(src) }))
    );
    const ok = await MS.modal({ title: `Propriétés de « ${src.name} »`, content, width: src.type === 'display_capture' ? 760 : 520 });
    if (ok) {
      Object.assign(src.settings, form.values());
      const name = nameInput.value.trim();
      if (name && name !== src.name && !src.global) src.name = name;
      MS.store.save();
      MS.runtimes.refresh(src);
      MS.emit('sources:changed');
    } else if (type.live) {
      src.settings = backup;
      MS.runtimes.refresh(src);
    }
    return ok;
  };

  /* Filtres d'une source -------------------------------------------------- */

  MS.openFilters = async function (sourceId) {
    const src = MS.store.source(sourceId);
    if (!src) return;
    const type = MS.SourceTypes[src.type] || {};
    if (type.audioOnly) return MS.toast('Cette source est uniquement audio : pas de filtre vidéo.', 'info');
    const backup = src.filters ? { ...src.filters } : null;
    const values = { ...MS.filterDefaults(), ...(src.filters || {}) };
    const num = (key, label, min, max) => ({ key, type: 'number', label, min, max });
    const fields = [
      { type: 'section', label: 'Correction des couleurs' },
      { key: 'opacity', type: 'range', label: 'Opacité (%)', min: 0, max: 100 },
      { key: 'brightness', type: 'range', label: 'Luminosité (%)', min: 0, max: 200 },
      { key: 'contrast', type: 'range', label: 'Contraste (%)', min: 0, max: 200 },
      { key: 'saturation', type: 'range', label: 'Saturation (%)', min: 0, max: 300 },
      { key: 'hue', type: 'range', label: 'Teinte (°)', min: -180, max: 180 },
      { type: 'section', label: 'Recadrage (pixels)' },
      num('cropLeft', 'Gauche', 0, 8192),
      num('cropTop', 'Haut', 0, 8192),
      num('cropRight', 'Droite', 0, 8192),
      num('cropBottom', 'Bas', 0, 8192),
      { key: 'flipH', type: 'checkbox', label: 'Retourner horizontalement' },
      { key: 'flipV', type: 'checkbox', label: 'Retourner verticalement' },
      { type: 'section', label: 'Incrustation (fond vert)' },
      { key: 'chroma', type: 'checkbox', label: 'Activer l’incrustation' },
      { key: 'chromaColor', type: 'color', label: 'Couleur clé' },
      { key: 'similarity', type: 'range', label: 'Similarité', min: 1, max: 1000 },
      { key: 'smoothness', type: 'range', label: 'Lissage', min: 1, max: 1000 },
    ];
    const apply = (vals) => {
      src.filters = { ...MS.filterDefaults(), ...vals };
    };
    const form = MS.buildForm(fields, values, apply);
    const reset = el('button', {
      class: 'btn',
      text: 'Réinitialiser',
      onclick: () => {
        src.filters = null;
        close(false);
      },
    });
    let close = () => {};
    const ok = await MS.modal({
      title: `Filtres de « ${src.name} »`,
      content: el('div', { class: 'filters-form' }, form.element, reset),
      width: 560,
      onOpen: (c) => (close = c),
    });
    if (ok) {
      apply(form.values());
      const isDefault = JSON.stringify(src.filters) === JSON.stringify(MS.filterDefaults());
      if (isDefault) src.filters = null;
    } else if (src.filters !== null || !backup) {
      src.filters = backup;
    }
    MS.store.save();
    MS.emit('items:changed');
  };

  /* Paramètres ----------------------------------------------------------- */

  MS.openSettings = async function (initialTab = 'stream') {
    const st = MS.store.settings;
    const info = MS.appInfo;
    const outputsActive = MS.outputs.isAnyActive();
    const services = Object.entries(MS.STREAM_SERVICES).map(([value, s]) => ({ value, label: s.label }));
    const presets = ['ultrafast', 'superfast', 'veryfast', 'faster', 'fast', 'medium'].map((p) => ({ value: p, label: p }));
    const micOptions = [{ value: 'disabled', label: 'Désactivé' }, ...(await MS.deviceOptions('audioinput', true))];
    const resolutions = ['1280x720', '1600x900', '1920x1080', '2560x1440', '3840x2160', '1080x1920'].map((r) => ({ value: r, label: r.replace('x', ' × ') }));
    const currentRes = `${st.video.width}x${st.video.height}`;
    if (!resolutions.some((r) => r.value === currentRes)) resolutions.push({ value: currentRes, label: currentRes.replace('x', ' × ') });

    const tabs = {
      stream: {
        label: 'Stream',
        fields: [
          { key: 'service', type: 'select', label: 'Service', options: services },
          {
            type: 'info',
            text: `Le flux part vers ton Restream (${st.restream.url || 'adresse à renseigner'}) qui le relaie à Twitch et Kick. Adresse RTMP et clé d’ingestion sont récupérées automatiquement : réglages dans l’onglet Restream.`,
            showIf: (v) => v.service === 'restream',
          },
          { key: 'server', type: 'text', label: 'Serveur', showIf: (v) => v.service !== 'restream' },
          { key: 'key', type: 'password', label: 'Clé de stream', showIf: (v) => v.service !== 'restream' },
          { key: 'encoder', type: 'select', label: 'Encodeur vidéo', options: [{ value: 'auto', label: `Automatique (${(MS.encoders[0] || { label: 'x264' }).label})` }, ...MS.encoders.map((e) => ({ value: e.id, label: e.label }))] },
          { key: 'videoBitrate', type: 'number', label: 'Débit vidéo (kb/s)', min: 500, max: 50000, step: 100 },
          {
            type: 'info',
            text: '⚠ Twitch recommande 6000 kb/s maximum : au-delà, tes viewers sans transcodage risquent des saccades.',
            showIf: (v) => v.service === 'twitch' && Number(v.videoBitrate) > 6000,
          },
          {
            type: 'info',
            text: '⚠ Kick accepte 8000 kb/s maximum.',
            showIf: (v) => v.service === 'kick' && Number(v.videoBitrate) > 8000,
          },
          {
            type: 'info',
            text: '⚠ Avec Restream, ce débit part tel quel vers Twitch (6000 kb/s max conseillé) et Kick (8000 max).',
            showIf: (v) => v.service === 'restream' && Number(v.videoBitrate) > 6000,
          },
          { key: 'audioBitrate', type: 'select', label: 'Débit audio (kb/s)', options: [96, 128, 160, 192, 256, 320].map((v) => ({ value: v, label: String(v) })) },
          {
            key: 'preset',
            type: 'select',
            label: 'Préréglage x264',
            options: presets,
            showIf: (v) => v.encoder === 'x264' || (v.encoder === 'auto' && (MS.encoders[0] || { id: 'x264' }).id === 'x264'),
          },
        ],
        values: { ...st.stream },
      },
      restream: {
        label: 'Restream',
        fields: [
          { type: 'info', text: 'Ton application Restream relaie le flux vers Twitch et Kick sans réencodage, et regroupe leur chat.' },
          { key: 'url', type: 'text', label: 'Adresse de l’interface Restream', placeholder: 'http://localhost:3050' },
          { key: 'password', type: 'password', label: 'Mot de passe (WEB_PASSWORD, si défini)' },
          { key: 'allowSelfSigned', type: 'checkbox', label: 'Accepter un certificat HTTPS auto-signé' },
          {
            key: 'rtmpUrl',
            type: 'text',
            label: 'Adresse RTMP (seulement si différente de l’interface)',
            placeholder: 'automatique : rtmp://<même hôte>:1935/live',
          },
          {
            type: 'button',
            label: 'Tester la connexion',
            onClick: async (vals, btn) => {
              btn.disabled = true;
              const r = await window.api.restreamTest(vals);
              btn.disabled = false;
              if (!r.ok) return MS.toast(`Restream : ${r.error}`, 'error', 8000);
              const d = Object.entries(r.status.destinations).map(([k, v]) => `${k} : ${v.status}`).join(', ');
              MS.toast(`Restream OK\nIngestion : ${r.ingest}\n${d || 'aucune destination'}`, 'success', 8000);
            },
          },
        ],
        values: { ...st.restream },
      },
      chat: {
        label: 'Chat',
        fields: [
          {
            key: 'mode',
            type: 'select',
            label: 'Source du chat',
            options: [
              { value: 'off', label: 'Désactivé' },
              { value: 'restream', label: 'Restream (Twitch + Kick, avec badges)' },
              { value: 'direct', label: 'Direct (Twitch uniquement, sans compte)' },
            ],
          },
          {
            type: 'info',
            text: 'Restream regroupe Twitch et Kick via leurs API officielles (badges et emotes inclus). Pour Kick, Restream doit être joignable en HTTPS (voir son README).',
            showIf: (v) => v.mode === 'restream',
          },
          { key: 'twitchChannel', type: 'text', label: 'Chaîne Twitch', placeholder: 'ex. m8_thedemon', showIf: (v) => v.mode === 'direct' },
          {
            type: 'info',
            text: 'Le mode direct lit uniquement le chat Twitch. Kick n’offre pas de lecture publique fiable : utilise le mode Restream pour l’avoir.',
            showIf: (v) => v.mode === 'direct',
          },
        ],
        values: { ...st.chat },
      },
      output: {
        label: 'Enregistrement',
        fields: [
          { key: 'recordPath', type: 'directory', label: "Dossier d'enregistrement", placeholder: info.videosPath },
          {
            key: 'format',
            type: 'select',
            label: 'Format',
            options: [
              { value: 'mp4', label: 'MP4 (H.264 / AAC)' },
              { value: 'mkv', label: 'MKV (H.264 / Opus)' },
              { value: 'webm', label: 'WebM (VP9 / Opus)' },
            ],
          },
          { key: 'videoBitrate', type: 'number', label: 'Débit vidéo (kb/s)', min: 500, max: 100000, step: 500 },
          { key: 'audioBitrate', type: 'select', label: 'Débit audio (kb/s)', options: [128, 160, 192, 256, 320].map((v) => ({ value: v, label: String(v) })) },
          { key: 'preset', type: 'select', label: 'Préréglage x264 (si réencodage)', options: presets },
        ],
        values: { ...st.output },
      },
      audio: {
        label: 'Audio',
        fields: [
          { key: 'micDevice', type: 'select', label: 'Micro / Auxiliaire', options: micOptions },
          { key: 'desktopAudio', type: 'checkbox', label: 'Capturer l’audio du bureau (Windows)' },
        ],
        values: { ...st.audio },
      },
      general: {
        label: 'Général',
        fields: [
          { key: 'confirmStopStream', type: 'checkbox', label: 'Demander confirmation avant d’arrêter le streaming' },
          { key: 'minimizeToTray', type: 'checkbox', label: 'Réduire dans la zone de notification' },
          { type: 'section', label: 'Raccourcis clavier (fonctionnent même en jeu)' },
          ...Object.entries(MS.HOTKEY_LABELS).map(([key, label]) => ({ key: `hk_${key}`, type: 'hotkey', label })),
        ],
        values: {
          confirmStopStream: st.ui.confirmStopStream,
          minimizeToTray: st.ui.minimizeToTray,
          ...Object.fromEntries(Object.keys(MS.HOTKEY_LABELS).map((k) => [`hk_${k}`, (st.ui.hotkeys || {})[k] || ''])),
        },
      },
      video: {
        label: 'Vidéo',
        fields: [
          outputsActive ? { type: 'info', text: 'Arrêtez le streaming et l’enregistrement pour modifier la vidéo.' } : null,
          { key: 'resolution', type: 'select', label: 'Résolution de base (canevas)', options: resolutions },
          { key: 'fps', type: 'select', label: 'Images par seconde', options: [24, 25, 30, 48, 50, 60].map((v) => ({ value: v, label: String(v) })) },
        ].filter(Boolean),
        values: { resolution: currentRes, fps: st.video.fps },
      },
    };

    const forms = {};
    const panes = {};
    const nav = el('div', { class: 'tabs' });
    const body = el('div', { class: 'tab-body' });
    let current = 'stream';
    for (const [key, tab] of Object.entries(tabs)) {
      forms[key] = MS.buildForm(tab.fields, tab.values, key === 'stream' ? onStreamChange : null);
      panes[key] = el('div', { class: 'tab-pane' }, forms[key].element);
      body.append(panes[key]);
      nav.append(el('button', { class: 'tab', text: tab.label, dataset: { tab: key }, onclick: () => show(key) }));
    }
    function show(key) {
      current = key;
      for (const [k, p] of Object.entries(panes)) p.hidden = k !== key;
      nav.querySelectorAll('.tab').forEach((b) => b.classList.toggle('active', b.dataset.tab === key));
    }
    let lastService = st.stream.service;
    function onStreamChange(vals) {
      if (vals.service !== lastService) {
        lastService = vals.service;
        const server = MS.STREAM_SERVICES[vals.service].server;
        const input = panes.stream.querySelectorAll('input[type="text"]')[0];
        if (input && server) input.value = server;
      }
    }
    show(tabs[initialTab] ? initialTab : 'stream');

    /** Remet l'onglet affiché à ses valeurs d'origine (appliquées seulement avec OK). */
    function resetTab() {
      const d = MS.defaultSettings();
      const defaults = {
        stream: d.stream,
        output: d.output,
        restream: d.restream,
        chat: d.chat,
        audio: d.audio,
        video: { resolution: `${d.video.width}x${d.video.height}`, fps: d.video.fps },
        general: {
          confirmStopStream: d.ui.confirmStopStream,
          minimizeToTray: d.ui.minimizeToTray,
          ...Object.fromEntries(Object.keys(MS.HOTKEY_LABELS).map((k) => [`hk_${k}`, ''])),
        },
      };
      forms[current] = MS.buildForm(tabs[current].fields, { ...defaults[current] }, current === 'stream' ? onStreamChange : null);
      panes[current].replaceChildren(forms[current].element);
      MS.toast(`Onglet « ${tabs[current].label} » remis par défaut (OK pour appliquer).`, 'info', 4000);
    }

    const ok = await MS.modal({
      title: 'Paramètres',
      content: el('div', { class: 'settings' }, nav, body),
      width: 720,
      buttons: [
        { label: 'Valeurs par défaut', left: true, onClick: resetTab },
        { label: 'Annuler', value: false },
        { label: 'OK', value: true, primary: true },
      ],
    });
    if (!ok) return;

    Object.assign(st.stream, forms.stream.values(), { audioBitrate: Number(forms.stream.values().audioBitrate) });
    Object.assign(st.output, forms.output.values(), { audioBitrate: Number(forms.output.values().audioBitrate) });
    Object.assign(st.restream, forms.restream.values());
    Object.assign(st.chat, forms.chat.values());
    const g = forms.general.values();
    st.ui.confirmStopStream = g.confirmStopStream;
    st.ui.minimizeToTray = g.minimizeToTray;
    const hotkeysBefore = JSON.stringify(st.ui.hotkeys);
    st.ui.hotkeys = Object.fromEntries(Object.keys(MS.HOTKEY_LABELS).map((k) => [k, g[`hk_${k}`]]));
    if (JSON.stringify(st.ui.hotkeys) !== hotkeysBefore) MS.hotkeys.apply();
    const a = forms.audio.values();
    Object.assign(st.audio, a);
    const v = forms.video.values();
    const [w, h] = v.resolution.split('x').map(Number);
    const fps = Number(v.fps);
    if (w !== st.video.width || h !== st.video.height || fps !== st.video.fps) {
      if (MS.outputs.isAnyActive()) {
        MS.toast('Les paramètres vidéo ne peuvent pas changer pendant une sortie active.', 'error');
      } else {
        Object.assign(st.video, { width: w, height: h, fps });
        MS.outputs.resetStream();
        MS.compositor.applyVideoSettings();
      }
    }
    MS.store.save();
    MS.syncGlobalAudio();
    MS.emit('settings:changed');
  };
})();
