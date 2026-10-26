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
            btns.map((b) => el('button', { class: b.primary ? 'btn primary' : 'btn', text: b.label, onclick: () => close(b.value) }))
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
    const changed = () => onChange && onChange(collect());
    const collect = () => {
      const out = {};
      for (const [k, g] of Object.entries(getters)) out[k] = g();
      return out;
    };

    for (const f of fields) {
      if (f.type === 'info') {
        root.append(el('p', { class: 'hint', text: f.text }));
        continue;
      }
      if (f.type === 'section') {
        root.append(el('div', { class: 'form-section', text: f.label }));
        continue;
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
          continue;
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
          input = el('input', { class: 'input', type: 'text', value: v ?? '', oninput: changed });
          getters[f.key] = () => input.value;
      }
      root.append(el('label', { class: `field ${f.type === 'capture' ? 'wide' : ''}` }, el('span', { text: f.label }), input));
    }
    return { element: root, values: collect };
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
      form.element
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

  /* Paramètres ----------------------------------------------------------- */

  MS.openSettings = async function () {
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
          { key: 'server', type: 'text', label: 'Serveur' },
          { key: 'key', type: 'password', label: 'Clé de stream' },
          { key: 'videoBitrate', type: 'number', label: 'Débit vidéo (kb/s)', min: 500, max: 50000, step: 100 },
          { key: 'audioBitrate', type: 'select', label: 'Débit audio (kb/s)', options: [96, 128, 160, 192, 256, 320].map((v) => ({ value: v, label: String(v) })) },
          { key: 'preset', type: 'select', label: 'Préréglage x264', options: presets },
        ],
        values: { ...st.stream },
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
    for (const [key, tab] of Object.entries(tabs)) {
      forms[key] = MS.buildForm(tab.fields, tab.values, key === 'stream' ? onStreamChange : null);
      panes[key] = el('div', { class: 'tab-pane' }, forms[key].element);
      body.append(panes[key]);
      nav.append(el('button', { class: 'tab', text: tab.label, dataset: { tab: key }, onclick: () => show(key) }));
    }
    function show(key) {
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
    show('stream');

    const ok = await MS.modal({ title: 'Paramètres', content: el('div', { class: 'settings' }, nav, body), width: 720 });
    if (!ok) return;

    Object.assign(st.stream, forms.stream.values(), { audioBitrate: Number(forms.stream.values().audioBitrate) });
    Object.assign(st.output, forms.output.values(), { audioBitrate: Number(forms.output.values().audioBitrate) });
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
