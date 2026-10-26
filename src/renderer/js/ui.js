'use strict';

(function () {
  const MS = window.MS;
  const el = MS.el;
  const store = () => MS.store;

  const ui = {
    init() {
      this.sceneList = MS.$('#scene-list');
      this.sourceList = MS.$('#source-list');
      this.mixer = MS.$('#mixer');
      this.ctxMenu = MS.$('#ctx-menu');

      this.bindToolbar();
      this.renderTransitions();

      MS.on('scenes:changed', () => this.renderScenes());
      MS.on('scene:switched', () => {
        this.renderScenes();
        this.renderSources();
        this.renderMixer();
      });
      for (const evt of ['items:changed', 'sources:changed', 'selection:changed', 'runtime:error', 'runtime:ready']) MS.on(evt, () => this.renderSources());
      for (const evt of ['mixer:changed', 'items:changed', 'sources:changed']) MS.on(evt, () => this.renderMixer());
      MS.on('outputs:changed', () => this.renderControls());

      document.addEventListener('pointerdown', (e) => {
        if (!this.ctxMenu.contains(e.target)) this.hideMenu();
      });
      window.addEventListener('blur', () => this.hideMenu());

      this.renderScenes();
      this.renderSources();
      this.renderMixer();
      this.renderControls();
      this.meterLoop();
      this.statusLoop();
    },

    bindToolbar() {
      const actions = {
        'scene-add': async () => {
          const name = await MS.prompt('Ajouter une scène', 'Nom de la scène', store().uniqueName('Scène', store().data.scenes.map((s) => s.name)));
          if (name) MS.compositor.switchScene(store().addScene(name).id);
        },
        'scene-remove': async () => {
          const scene = store().currentScene();
          if (store().data.scenes.length <= 1) return MS.toast('Il doit rester au moins une scène.', 'error');
          if (await MS.confirm('Supprimer la scène', `Supprimer « ${scene.name} » ?`, 'Supprimer')) store().removeScene(scene.id);
        },
        'scene-up': () => store().moveScene(store().data.currentSceneId, -1),
        'scene-down': () => store().moveScene(store().data.currentSceneId, 1),
        'source-add': (e) => this.addSourceMenu(e.currentTarget),
        'source-remove': () => {
          const item = MS.preview.selectedItem();
          if (item) this.removeItem(item);
        },
        'source-props': () => {
          const item = MS.preview.selectedItem();
          if (item) MS.openProperties(item.sourceId);
        },
        'source-up': () => this.moveSelected(-1),
        'source-down': () => this.moveSelected(1),
        stream: () => MS.outputs.toggle('stream'),
        record: () => MS.outputs.toggle('record'),
        settings: () => MS.openSettings(),
        folder: () => window.api.openPath(store().settings.output.recordPath || MS.appInfo.videosPath),
        quit: () => window.close(),
      };
      document.querySelectorAll('[data-act]').forEach((b) => {
        b.addEventListener('click', (e) => actions[b.dataset.act] && actions[b.dataset.act](e));
      });
    },

    moveSelected(delta) {
      const item = MS.preview.selectedItem();
      if (item) store().moveItem(store().data.currentSceneId, item.id, delta);
    },

    async removeItem(item) {
      const src = store().source(item.sourceId);
      const ok = await MS.confirm('Supprimer la source', `Supprimer « ${src ? src.name : 'source'} » de la scène ?`, 'Supprimer');
      if (!ok) return;
      MS.preview.select(null);
      store().removeItem(store().data.currentSceneId, item.id);
    },

    /* Scènes ----------------------------------------------------------- */
    renderScenes() {
      const cur = store().data.currentSceneId;
      this.sceneList.replaceChildren(
        ...store().data.scenes.map((s) =>
          el(
            'li',
            {
              class: s.id === cur ? 'active' : '',
              onclick: () => MS.compositor.switchScene(s.id),
              ondblclick: async () => {
                const name = await MS.prompt('Renommer la scène', 'Nom', s.name);
                if (name) store().renameScene(s.id, name);
              },
            },
            el('span', { class: 'name', text: s.name })
          )
        )
      );
      MS.$('#scene-label').textContent = store().currentScene() ? store().currentScene().name : '';
    },

    /* Sources ---------------------------------------------------------- */
    renderSources() {
      const scene = store().currentScene();
      if (!scene) return;
      const sel = MS.preview.selectedId;
      if (!scene.items.length) {
        this.sourceList.replaceChildren(el('li', { class: 'empty', text: 'Cliquez sur + pour ajouter une source.' }));
        return;
      }
      this.sourceList.replaceChildren(
        ...scene.items.map((item) => {
          const src = store().source(item.sourceId);
          if (!src) return null;
          const type = MS.SourceTypes[src.type] || {};
          const rt = MS.runtimes.get(src.id);
          const toggle = (key, on, off, title) =>
            el(
              'button',
              {
                class: `icon-btn ${item[key] ? '' : 'off'}`,
                title,
                onclick: (e) => {
                  e.stopPropagation();
                  store().updateItem(scene.id, item.id, { [key]: !item[key] });
                },
              },
              MS.icon(item[key] ? on : off)
            );
          return el(
            'li',
            {
              class: `${item.id === sel ? 'active' : ''} ${item.visible ? '' : 'hidden-item'}`,
              onclick: () => MS.preview.select(item.id),
              ondblclick: () => MS.openProperties(src.id),
              oncontextmenu: (e) => {
                e.preventDefault();
                MS.preview.select(item.id);
                this.itemContextMenu(item, e.clientX, e.clientY);
              },
            },
            el('span', { class: 'glyph', text: type.glyph || '•' }),
            el('span', { class: 'name', text: src.name }),
            rt && rt.error ? el('span', { class: 'warn', title: rt.error }, MS.icon('warning')) : null,
            toggle('visible', 'eye', 'eyeOff', 'Afficher / masquer'),
            el(
              'button',
              {
                class: `icon-btn ${item.locked ? '' : 'off'}`,
                title: 'Verrouiller',
                onclick: (e) => {
                  e.stopPropagation();
                  store().updateItem(scene.id, item.id, { locked: !item.locked });
                },
              },
              MS.icon(item.locked ? 'lock' : 'unlock')
            )
          );
        })
      );
    },

    addSourceMenu(anchor) {
      const r = anchor.getBoundingClientRect();
      const platform = MS.appInfo.platform;
      const entries = Object.entries(MS.SourceTypes)
        .filter(([key]) => key !== 'desktop_audio' || platform === 'win32')
        .map(([key, t]) => ({ label: `${t.glyph}  ${t.label}`, action: () => this.createSource(key) }));
      const scene = store().currentScene();
      const existing = Object.values(store().data.sources).filter((s) => !s.global && !scene.items.some((it) => it.sourceId === s.id));
      if (existing.length) {
        entries.push({ separator: true });
        entries.push({ label: 'Ajouter une source existante', disabled: true });
        for (const s of existing) {
          entries.push({ label: `   ${(MS.SourceTypes[s.type] || {}).glyph || ''}  ${s.name}`, action: () => store().addItem(scene.id, s.id) });
        }
      }
      this.showMenu(entries, r.left, r.top, true);
    },

    async createSource(type) {
      const t = MS.SourceTypes[type];
      const name = await MS.prompt(`Ajouter : ${t.label}`, 'Nom de la source', store().uniqueName(t.label, Object.values(store().data.sources).map((s) => s.name)));
      if (!name) return;
      const src = store().createSource(type, name, t.defaults());
      const scene = store().currentScene();
      const item = store().addItem(scene.id, src.id);
      MS.preview.select(item.id);
      const ok = await MS.openProperties(src.id);
      if (!ok && type === 'display_capture' && !src.settings.captureId) store().removeItem(scene.id, item.id);
    },

    itemContextMenu(item, x, y) {
      const sceneId = store().data.currentSceneId;
      this.showMenu(
        [
          { label: 'Propriétés…', action: () => MS.openProperties(item.sourceId) },
          { label: 'Renommer…', action: () => this.renameSource(item.sourceId) },
          { separator: true },
          { label: "Ajuster à l'écran", action: () => MS.preview.transform(item, 'fit') },
          { label: "Étirer à l'écran", action: () => MS.preview.transform(item, 'stretch') },
          { label: "Centrer à l'écran", action: () => MS.preview.transform(item, 'center') },
          { label: 'Taille d’origine', action: () => MS.preview.transform(item, 'reset') },
          { separator: true },
          { label: 'Monter', action: () => store().moveItem(sceneId, item.id, -1) },
          { label: 'Descendre', action: () => store().moveItem(sceneId, item.id, 1) },
          { label: item.locked ? 'Déverrouiller' : 'Verrouiller', action: () => store().updateItem(sceneId, item.id, { locked: !item.locked }) },
          { separator: true },
          { label: 'Supprimer', action: () => this.removeItem(item) },
        ],
        x,
        y
      );
    },

    async renameSource(id) {
      const src = store().source(id);
      if (!src) return;
      const name = await MS.prompt('Renommer la source', 'Nom', src.name);
      if (name) store().updateSource(id, { name });
    },

    showMenu(entries, x, y, above) {
      const m = this.ctxMenu;
      m.replaceChildren(
        ...entries.map((e) =>
          e.separator
            ? el('div', { class: 'sep' })
            : el('button', {
                class: 'menu-item',
                text: e.label,
                disabled: Boolean(e.disabled),
                onclick: () => {
                  this.hideMenu();
                  e.action && e.action();
                },
              })
        )
      );
      m.classList.remove('hidden');
      const mw = m.offsetWidth;
      const mh = m.offsetHeight;
      m.style.left = `${Math.min(x, window.innerWidth - mw - 4)}px`;
      m.style.top = `${above ? Math.max(4, y - mh - 4) : Math.min(y, window.innerHeight - mh - 4)}px`;
    },

    hideMenu() {
      this.ctxMenu.classList.add('hidden');
    },

    /* Mixeur audio ----------------------------------------------------- */
    renderMixer() {
      const list = MS.audio.mixerSources();
      if (!list.length) {
        this.mixer.replaceChildren(el('div', { class: 'empty', text: 'Aucune source audio.' }));
        return;
      }
      this.mixer.replaceChildren(
        ...list.map((src) => {
          const db = el('span', { class: 'db', text: '' });
          const fader = el('input', {
            type: 'range',
            min: 0,
            max: 1000,
            value: Math.round(MS.audio.gainToFader(src.volume) * 1000),
            oninput: () => {
              src.volume = MS.audio.faderToGain(fader.value / 1000);
              db.textContent = gainLabel(src.volume);
              MS.audio.updateGains();
              MS.store.save();
            },
          });
          db.textContent = gainLabel(src.volume);
          const mute = el(
            'button',
            {
              class: `icon-btn ${src.muted ? 'danger' : ''}`,
              title: 'Muet',
              onclick: () => {
                src.muted = !src.muted;
                MS.audio.updateGains();
                MS.store.save();
                this.renderMixer();
              },
            },
            MS.icon(src.muted ? 'mute' : 'speaker')
          );
          const monitor = el(
            'button',
            {
              class: `icon-btn ${src.monitor ? 'on' : 'off'}`,
              title: 'Écouter (monitoring)',
              onclick: () => {
                src.monitor = !src.monitor;
                MS.audio.updateGains();
                MS.store.save();
                this.renderMixer();
              },
            },
            MS.icon('headphones')
          );
          const props = el('button', { class: 'icon-btn off', title: 'Propriétés', onclick: () => MS.openProperties(src.id) }, MS.icon('gear'));
          return el(
            'div',
            { class: `channel ${src.muted ? 'muted' : ''}`, dataset: { id: src.id } },
            el('div', { class: 'channel-head' }, el('span', { class: 'name', text: src.name }), db),
            el('div', { class: 'meter' }, el('div', { class: 'meter-cover' }), el('div', { class: 'meter-ticks' })),
            el('div', { class: 'channel-controls' }, fader, mute, monitor, props)
          );
        })
      );
    },

    meterLoop() {
      const tick = () => {
        for (const ch of this.mixer.querySelectorAll('.channel')) {
          const db = MS.audio.level(ch.dataset.id);
          const level = Number.isFinite(db) ? MS.clamp((db + 60) / 60, 0, 1) : 0;
          ch.querySelector('.meter-cover').style.width = `${(1 - level) * 100}%`;
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    },

    /* Transitions ------------------------------------------------------ */
    renderTransitions() {
      const t = store().settings.transition;
      const type = MS.$('#transition-type');
      const dur = MS.$('#transition-duration');
      type.value = t.type;
      dur.value = t.duration;
      dur.disabled = t.type === 'cut';
      type.addEventListener('change', () => {
        t.type = type.value;
        dur.disabled = t.type === 'cut';
        store().save();
      });
      dur.addEventListener('change', () => {
        t.duration = MS.clamp(Number(dur.value) || 0, 0, 10000);
        dur.value = t.duration;
        store().save();
      });
    },

    /* Commandes et barre d'état --------------------------------------- */
    renderControls() {
      const o = MS.outputs;
      const setBtn = (id, kind, on, off) => {
        const b = MS.$(id);
        const active = Boolean(o.active[kind]);
        b.textContent = o.busy[kind] ? (active ? 'Arrêt…' : 'Démarrage…') : active ? on : off;
        b.classList.toggle('live', active);
        b.disabled = o.busy[kind];
      };
      setBtn('#btn-stream', 'stream', 'Arrêter le streaming', 'Démarrer le streaming');
      setBtn('#btn-record', 'record', "Arrêter l'enregistrement", "Démarrer l'enregistrement");
    },

    statusLoop() {
      const live = MS.$('#status-live');
      const rec = MS.$('#status-rec');
      const perf = MS.$('#status-perf');
      const fps = MS.$('#status-fps');
      let cpu = null;
      setInterval(async () => {
        try {
          cpu = await window.api.stats();
        } catch {
          cpu = null;
        }
      }, 2000);
      setInterval(() => {
        const { stream, record } = MS.outputs.active;
        const now = performance.now();
        live.classList.toggle('on', Boolean(stream));
        rec.classList.toggle('on', Boolean(record));
        let liveText = `LIVE ${stream ? MS.formatTime(now - stream.startedAt) : '00:00:00'}`;
        if (stream && stream.progress && stream.progress.bitrate) liveText += ` · ${Math.round(stream.progress.bitrate)} kb/s`;
        live.textContent = liveText;
        rec.textContent = `REC ${record ? MS.formatTime(now - record.startedAt) : '00:00:00'}`;
        perf.textContent = cpu ? `CPU ${cpu.cpu.toFixed(1)} % · ${Math.round(cpu.memMB)} Mo` : '';
        const s = MS.compositor.stats;
        const target = store().settings.video.fps;
        fps.textContent = `${s.fps.toFixed(1)} / ${target} FPS · rendu ${s.renderMs.toFixed(1)} ms · images en retard ${s.lagged}`;
      }, 500);
    },
  };

  function gainLabel(g) {
    return g <= 0 ? '-∞ dB' : `${(20 * Math.log10(g)).toFixed(1)} dB`;
  }

  MS.ui = ui;
})();
