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

      this.chatList = MS.$('#chat-list');
      this.bindToolbar();
      this.bindMenus();
      this.renderTransitions();
      MS.layout.init();

      MS.on('scenes:changed', () => this.renderScenes());
      MS.on('scene:switched', () => {
        this.renderScenes();
        this.renderSources();
        this.renderMixer();
      });
      for (const evt of ['preview:switched', 'studio:changed']) {
        MS.on(evt, () => {
          this.renderScenes();
          this.renderSources();
          this.renderControls();
        });
      }
      for (const evt of ['selection:changed', 'items:changed', 'sources:changed', 'scene:switched', 'preview:switched']) MS.on(evt, () => this.renderPreviewBar());
      MS.on('chat:message', (m) => this.appendChat(m));
      MS.on('chat:cleared', () => this.renderChat());
      MS.on('chat:status', () => this.renderChatStatus());
      MS.on('settings:changed', () => this.renderChatStatus());
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
      this.renderPreviewBar();
      this.renderChat();
      this.renderChatStatus();
      this.meterLoop();
      this.statusLoop();
      this.initUpdates();
    },

    bindToolbar() {
      const actions = {
        'scene-add': async () => {
          const name = await MS.prompt('Ajouter une scène', 'Nom de la scène', store().uniqueName('Scène', store().data.scenes.map((s) => s.name)));
          if (name) this.activateScene(store().addScene(name).id);
        },
        'scene-remove': async () => {
          const scene = store().editScene();
          if (store().data.scenes.length <= 1) return MS.toast('Il doit rester au moins une scène.', 'error');
          if (await MS.confirm('Supprimer la scène', `Supprimer « ${scene.name} » ?`, 'Supprimer')) store().removeScene(scene.id);
        },
        'scene-up': () => store().moveScene(store().editScene().id, -1),
        'scene-down': () => store().moveScene(store().editScene().id, 1),
        'source-add': (e) => this.addSourceMenu(e.currentTarget),
        'source-remove': () => {
          const item = MS.preview.selectedItem();
          if (item) this.removeItem(item);
        },
        'source-props': () => {
          const item = MS.preview.selectedItem();
          if (item) MS.openProperties(item.sourceId);
        },
        'source-filters': () => {
          const item = MS.preview.selectedItem();
          if (item) MS.openFilters(item.sourceId);
        },
        studio: () => store().setStudio(!store().settings.ui.studio),
        transition: () => MS.compositor.switchScene(store().data.previewSceneId),
        'chat-clear': () => MS.chat.clear(),
        'chat-settings': () => MS.openSettings('chat'),
        'source-up': () => this.moveSelected(-1),
        'source-down': () => this.moveSelected(1),
        stream: async () => {
          // Comme le demandent beaucoup d'utilisateurs d'OBS : pas d'arrêt du live par un clic malheureux.
          const live = MS.outputs.active.stream || MS.outputs.reconnect;
          if (live && store().settings.ui.confirmStopStream && !(await MS.confirm('Arrêter le streaming', 'Arrêter le live maintenant ?', 'Arrêter'))) return;
          MS.outputs.toggle('stream');
        },
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
      if (item) store().moveItem(store().editScene().id, item.id, delta);
    },

    async removeItem(item) {
      const src = store().source(item.sourceId);
      const ok = await MS.confirm('Supprimer la source', `Supprimer « ${src ? src.name : 'source'} » de la scène ?`, 'Supprimer');
      if (!ok) return;
      MS.preview.select(null);
      store().removeItem(store().editScene().id, item.id);
    },

    duplicateScene(id) {
      const copy = store().duplicateScene(id);
      if (copy) this.activateScene(copy.id);
    },

    /** Clic sur une scène : en mode studio elle va dans l'aperçu, sinon directement à l'antenne. */
    activateScene(id) {
      if (store().settings.ui.studio) store().setPreviewScene(id);
      else MS.compositor.switchScene(id);
    },

    /* Scènes ----------------------------------------------------------- */
    renderScenes() {
      const program = store().data.currentSceneId;
      const studio = store().settings.ui.studio;
      const edit = store().editScene().id;
      const rename = async (s) => {
        const name = await MS.prompt('Renommer la scène', 'Nom', s.name);
        if (name) store().renameScene(s.id, name);
      };
      this.sceneList.replaceChildren(
        ...store().data.scenes.map((s) =>
          el(
            'li',
            {
              class: `${s.id === edit ? 'active' : ''} ${studio && s.id === program ? 'on-air' : ''}`,
              onclick: () => this.activateScene(s.id),
              // En mode studio, comme OBS : double-clic = passer la scène à l'antenne.
              ondblclick: () => (studio ? MS.compositor.switchScene(s.id) : rename(s)),
              oncontextmenu: (e) => {
                e.preventDefault();
                this.showMenu(
                  [
                    { label: 'Renommer…', action: () => rename(s) },
                    { label: 'Passer à l’antenne', action: () => MS.compositor.switchScene(s.id) },
                    { label: 'Dupliquer', action: () => this.duplicateScene(s.id) },
                    { label: `Transition vers cette scène… (${s.transition ? MS.TRANSITION_LABELS[s.transition.type] : 'par défaut'})`, action: () => MS.openTransitionOptions(s.id) },
                    { label: 'Supprimer', action: () => MS.$('[data-act="scene-remove"]').click() },
                  ],
                  e.clientX,
                  e.clientY
                );
              },
            },
            el('span', { class: 'name', text: s.name }),
            studio && s.id === program ? el('span', { class: 'live-dot', title: 'À l’antenne' }) : null
          )
        )
      );
      MS.$('#scene-label').textContent = store().currentScene() ? store().currentScene().name : '';
    },

    /* Sources ---------------------------------------------------------- */
    renderSources() {
      const scene = store().editScene();
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
            el('span', { class: 'name', text: src.name, title: rt && rt.status ? `${src.name} · ${rt.status}` : src.name }, rt && rt.status ? el('small', { class: 'src-status', text: rt.status }) : null),
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
        .filter(([key]) => !['desktop_audio', 'app_audio', 'game_capture'].includes(key) || platform === 'win32')
        .map(([key, t]) => ({ label: `${t.glyph}  ${t.label}`, action: () => this.createSource(key) }));
      const scene = store().editScene();
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
      const scene = store().editScene();
      const item = store().addItem(scene.id, src.id);
      MS.preview.select(item.id);
      const ok = await MS.openProperties(src.id);
      if (!ok && type === 'display_capture' && !src.settings.captureId) store().removeItem(scene.id, item.id);
    },

    itemContextMenu(item, x, y) {
      const sceneId = store().editScene().id;
      this.showMenu(
        [
          { label: 'Propriétés…', action: () => MS.openProperties(item.sourceId) },
          { label: 'Filtres…', action: () => MS.openFilters(item.sourceId) },
          { label: 'Renommer…', action: () => this.renameSource(item.sourceId) },
          { separator: true },
          { label: "Ajuster à l'écran", action: () => MS.preview.transform(item, 'fit') },
          { label: "Étirer à l'écran", action: () => MS.preview.transform(item, 'stretch') },
          { label: "Centrer à l'écran", action: () => MS.preview.transform(item, 'center') },
          { label: 'Taille d’origine', action: () => MS.preview.transform(item, 'reset') },
          { label: 'Modifier la transformation…', action: () => MS.openTransform(item) },
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
          const props = el(
            'button',
            {
              class: 'icon-btn off',
              title: 'Filtres audio et propriétés',
              onclick: (e) => {
                const r = e.currentTarget.getBoundingClientRect();
                this.showMenu(
                  [
                    { label: 'Filtres audio (gain, noise gate, compresseur)…', action: () => MS.openAudioFilters(src.id) },
                    { label: 'Propriétés…', action: () => MS.openProperties(src.id) },
                  ],
                  r.left,
                  r.bottom + 2
                );
              },
            },
            MS.icon('gear')
          );
          return el(
            'div',
            { class: `channel ${src.muted ? 'muted' : ''}`, dataset: { id: src.id } },
            el(
              'div',
              { class: 'channel-head' },
              el('span', { class: 'name', text: src.name }),
              el('span', { class: 'clip', text: 'SATURE' }),
              hasAudioFx(src) ? el('span', { class: 'fx', text: 'FX', title: 'Filtres audio actifs' }) : null,
              db
            ),
            el('div', { class: 'meter' }, el('div', { class: 'meter-cover' }), el('div', { class: 'meter-ticks' }, METER_TICKS.map((d) => el('i', { style: { left: `${dbPos(d) * 100}%` } }))), el('div', { class: 'meter-peak' })),
            el('div', { class: 'meter-scale' }, METER_TICKS.map((d) => el('span', { text: d === 0 ? '0' : String(d).replace('-', '−'), style: { left: `${dbPos(d) * 100}%` } }))),
            el('div', { class: 'channel-controls' }, fader, mute, monitor, props)
          );
        })
      );
    },

    /**
     * VU-mètres à 30 Hz (largement suffisant à l'œil), seulement si le dock
     * est visible. La crête reste affichée 1,5 s puis redescend ; une source
     * qui sature reste signalée 3 s pour qu'on ait le temps de le voir.
     */
    meterLoop() {
      const holds = new Map();
      setInterval(() => {
        if (MS.$('#dock-mixer').hidden) return;
        const now = performance.now();
        for (const ch of this.mixer.querySelectorAll('.channel')) {
          const id = ch.dataset.id;
          const db = MS.audio.level(id);
          const level = Number.isFinite(db) ? dbPos(db) : 0;
          ch.querySelector('.meter-cover').style.transform = `scaleX(${1 - level})`;
          let h = holds.get(id);
          if (!h || level >= h.level) h = { level, at: now, clipUntil: h ? h.clipUntil : 0 };
          else if (now - h.at > 1500) h.level = Math.max(level, h.level - 0.012);
          if (Number.isFinite(db) && db >= -0.5) h.clipUntil = now + 3000;
          holds.set(id, h);
          const peak = ch.querySelector('.meter-peak');
          peak.style.left = `calc(${h.level * 100}% - 2px)`;
          peak.style.opacity = h.level > 0.01 ? '1' : '0';
          ch.classList.toggle('clipping', h.clipUntil > now);
        }
      }, 33);
    },

    /* Transitions ------------------------------------------------------ */
    renderTransitions() {
      const t = store().settings.transition;
      const type = MS.$('#transition-type');
      const dur = MS.$('#transition-duration');
      const refresh = () => {
        type.value = t.type;
        dur.value = t.duration;
        // Le stinger dure le temps de sa vidéo ; la coupure est instantanée.
        MS.$('#transition-duration-field').hidden = t.type === 'cut' || t.type === 'stinger';
        MS.$('#transition-options').hidden = t.type !== 'slide' && t.type !== 'stinger';
        MS.$('#transition-summary').textContent = MS.transitionSummary(t);
      };
      type.addEventListener('change', () => {
        t.type = type.value;
        store().save();
        refresh();
        MS.compositor.preloadStingers();
        if (t.type === 'stinger' && !t.stingerFile) MS.openTransitionOptions();
      });
      dur.addEventListener('change', () => {
        t.duration = MS.clamp(Number(dur.value) || 0, 0, 10000);
        store().save();
        refresh();
      });
      MS.$('#transition-options').addEventListener('click', () => MS.openTransitionOptions());
      MS.on('transition:changed', refresh);
      refresh();
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
      if (o.reconnect) {
        const b = MS.$('#btn-stream');
        b.textContent = `Reconnexion ${o.reconnect.attempt}… (cliquer pour arrêter)`;
        b.classList.add('live');
      }
      setBtn('#btn-record', 'record', "Arrêter l'enregistrement", "Démarrer l'enregistrement");
      MS.$('#btn-studio').classList.toggle('on', store().settings.ui.studio);
    },

    /* Barre sous l'aperçu ----------------------------------------------- */
    renderPreviewBar() {
      const item = MS.preview.selectedItem();
      const src = item && store().source(item.sourceId);
      MS.$('#selected-label').textContent = src ? src.name : 'Aucune source sélectionnée';
      for (const b of document.querySelectorAll('#preview-toolbar .btn')) b.disabled = !src;
    },

    /* Chat ----------------------------------------------------------------- */
    chatLine(m) {
      if (m.divider) return el('div', { class: 'chat-divider', text: m.message });
      if (m.system) return el('div', { class: 'chat-line chat-system', text: `[${m.platform}] ${m.message}` });
      const icon = MS.PLATFORM_ICONS[m.platform]
        ? el('span', {
            class: `chat-icon ${m.platform}`,
            title: m.platform,
            html: `<svg viewBox="0 0 24 24"><path d="${MS.PLATFORM_ICONS[m.platform]}"/></svg>`,
          })
        : null;
      const badges = (m.badges || []).map((b) => {
        const label = MS.BADGE_LABELS[b.type] || b.title || b.type;
        const title = b.count ? `${label} (${b.count})` : label;
        if (b.imageUrl) return el('img', { class: 'chat-badge-img', src: b.imageUrl, alt: label, title });
        const ic = MS.BADGE_ICONS[b.type];
        if (ic) {
          return el('span', {
            class: 'chat-badge-img',
            title,
            html: `<svg viewBox="0 0 24 24"><rect width="24" height="24" rx="5" fill="${ic.bg}"/><path d="${ic.d}" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/></svg>`,
          });
        }
        return el('span', { class: 'chat-badge', text: label, title });
      });
      const parts = (m.parts || [{ type: 'text', text: m.message }]).map((p) =>
        p.type === 'emote' ? el('img', { class: 'chat-emote', src: p.url, alt: p.name, title: p.name }) : document.createTextNode(p.text)
      );
      return el(
        'div',
        { class: `chat-line ${m.mentioned ? 'mentioned' : ''}` },
        icon,
        badges,
        el('span', { class: 'chat-user', text: m.username, style: m.color ? { color: m.color } : null }),
        el('span', { class: 'chat-text' }, ' ', parts)
      );
    },

    appendChat(m) {
      const list = this.chatList;
      const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
      list.querySelector('.empty')?.remove();
      list.append(this.chatLine(m));
      while (list.children.length > 200) list.firstChild.remove();
      if (atBottom) list.scrollTop = list.scrollHeight;
    },

    renderChat() {
      const msgs = MS.chat.messages;
      this.chatList.replaceChildren(...(msgs.length ? msgs.map((m) => this.chatLine(m)) : [el('div', { class: 'empty', text: 'Aucun message pour le moment.' })]));
      this.chatList.scrollTop = this.chatList.scrollHeight;
    },

    renderChatStatus() {
      const mode = store().settings.chat.mode;
      const box = MS.$('#chat-status');
      if (mode === 'off') {
        box.replaceChildren(el('button', { class: 'link', text: 'Configurer le chat…', onclick: () => MS.openSettings('chat') }));
        return;
      }
      const labels = { connected: 'connecté', connecting: 'connexion…', error: 'erreur' };
      const entries = Object.entries(MS.chat.status);
      const chips = (entries.length ? entries : [[mode, { status: 'connecting' }]]).map(([p, st]) =>
        el('span', { class: `chip ${st.status}`, title: st.error || '', text: `${p} : ${labels[st.status] || st.status}` })
      );
      const errors = entries.filter(([, st]) => st.status === 'error' && st.error).map(([p, st]) => el('div', { class: 'chat-error', text: `${p} : ${st.error}` }));
      const hint = null;
      box.replaceChildren(...chips, ...errors, hint || '');
    },

    /* Menus et docks ------------------------------------------------------- */
    bindMenus() {
      document.querySelectorAll('[data-menu]').forEach((b) => b.addEventListener('click', () => this.openMenu(b.dataset.menu, b)));
    },

    openMenu(name, anchor) {
      const r = anchor.getBoundingClientRect();
      const ui = store().settings.ui;
      const check = (on, label) => `${on ? '✓' : '   '}  ${label}`;
      const menus = {
        file: [
          { label: 'Afficher les enregistrements', action: () => MS.$('[data-act="folder"]').click() },
          { separator: true },
          { label: 'Exporter la collection de scènes…', action: () => MS.exportCollection() },
          { label: 'Importer une collection de scènes…', action: () => MS.importCollection(false) },
          { label: 'Importer une collection OBS…', action: () => MS.importObsCollection() },
          { label: 'Restaurer une sauvegarde automatique…', action: () => MS.importCollection(true) },
          { separator: true },
          { label: 'Titre et catégorie du live…', action: () => MS.openLiveInfo() },
          { label: 'Historique des lives…', action: () => MS.live.showHistory() },
          { separator: true },
          { label: 'Paramètres', action: () => MS.openSettings() },
          { separator: true },
          { label: 'Quitter', action: () => window.close() },
        ],
        edit: [
          { label: 'Annuler                      Ctrl+Z', action: () => MS.history.undo() },
          { label: 'Rétablir                      Ctrl+Y', action: () => MS.history.redo() },
          { separator: true },
          { label: 'Modifier la transformation   Ctrl+E', disabled: !MS.preview.selectedItem(), action: () => MS.openTransform(MS.preview.selectedItem()) },
          { label: 'Dupliquer la scène', action: () => this.duplicateScene(store().editScene().id) },
        ],
        view: [
          { label: check(ui.studio, 'Mode studio'), action: () => store().setStudio(!ui.studio) },
          { label: check(ui.docks.chat, 'Chat'), action: () => this.toggleDock('chat') },
          {
            label: check(ui.liveBar !== false, 'Barre « En direct » pendant le live'),
            action: () => {
              ui.liveBar = ui.liveBar === false;
              store().save();
              MS.live.render();
            },
          },
        ],
        docks: [...Object.entries(DOCK_LABELS).map(([k, label]) => ({ label: check(ui.docks[k], label), action: () => this.toggleDock(k) })), ...MS.layout.menuItems()],
        help: [
          { label: 'Assistant de configuration…', action: () => MS.openWizard() },
          { label: 'Rechercher des mises à jour', action: () => this.checkUpdates() },
          { label: 'Ouvrir le dossier des journaux', action: () => window.api.openLogs() },
          { separator: true },
          { label: 'Page du projet (GitHub)', action: () => window.api.openExternal('https://github.com/TheDEMON78/mystream') },
          { label: 'Nouveautés (releases)', action: () => window.api.openExternal('https://github.com/TheDEMON78/mystream/releases') },
          { separator: true },
          { label: `MyStream v${MS.appInfo.version} — BloopStudio`, disabled: true },
        ],
      };
      this.showMenu(menus[name], r.left, r.bottom + 2);
    },

    toggleDock(key) {
      const docks = store().settings.ui.docks;
      docks[key] = !docks[key];
      store().save();
      this.applyDocks();
    },

    applyDocks() {
      MS.layout.apply();
    },

    /* Mises à jour ---------------------------------------------------------- */
    initUpdates() {
      const badge = MS.$('#update-badge');
      badge.addEventListener('click', () => {
        if (MS.outputs.isAnyActive()) return MS.toast("La mise à jour s'installera après le live ou l'enregistrement : arrête-les d'abord.", 'info', 6000);
        window.api.updateInstall();
      });
      // Les mises à jour attendent la fin des lives et des enregistrements.
      MS.on('outputs:changed', () => window.api.setOutputsBusy(MS.outputs.isAnyActive()));
      const show = (st) => {
        const texts = {
          downloading: `Mise à jour v${st.version} : téléchargement ${st.percent || 0} %`,
          ready: `Redémarrer pour installer la v${st.version}`,
          available: `Nouvelle version v${st.version} : télécharger`,
          waiting: `Mise à jour v${st.version} : après le live`,
        };
        badge.hidden = !texts[st.status];
        badge.textContent = texts[st.status] || '';
        badge.disabled = st.status === 'downloading' || st.status === 'waiting';
        if (st.status === 'ready' && this.lastUpdate !== 'ready') MS.toast(`La v${st.version} est prête : elle s'installera à la fermeture de MyStream.`, 'success', 8000);
        if (this.manualCheck && ['latest', 'error', 'available', 'downloading', 'deferred', 'waiting'].includes(st.status)) {
          this.manualCheck = false;
          if (st.status === 'latest') MS.toast('MyStream est à jour.', 'success');
          else if (st.status === 'waiting') MS.toast(`La v${st.version} est prête à être téléchargée : ce sera fait une minute après la fin du live ou de l’enregistrement.`, 'info', 6000);
          else if (st.status === 'deferred') MS.toast('Recherche reportée : elle se fera une minute après la fin du live ou de l’enregistrement.', 'info', 6000);
          else if (st.status === 'error') MS.toast(`Mise à jour impossible : ${st.error}`, 'error', 8000);
        }
        this.lastUpdate = st.status;
      };
      window.api.onUpdateState(show);
      window.api.updateState().then(show);
    },

    checkUpdates() {
      this.manualCheck = true;
      MS.toast('Recherche de mises à jour…', 'info', 2500);
      window.api.updateCheck();
    },

    statusLoop() {
      const live = MS.$('#status-live');
      const rec = MS.$('#status-rec');
      const perf = MS.$('#status-perf');
      const fps = MS.$('#status-fps');
      const health = MS.$('#status-health');
      MS.on('health:changed', (h) => {
        health.className = `health ${h.level}`;
        health.title = h.message;
      });
      health.addEventListener('click', () => MS.toast(MS.outputs.health.message || 'Aucune sortie active.', 'info', 8000));
      const rs = MS.$('#status-restream');
      let cpu = null;
      const pollRestream = async () => {
        const st = store().settings;
        if (st.stream.service !== 'restream') {
          rs.hidden = true;
          return;
        }
        rs.hidden = false;
        const r = await window.api.restreamStatus(st.restream);
        if (!r.ok) {
          rs.className = 'badge error';
          rs.textContent = 'Restream injoignable';
          rs.title = r.error;
          return;
        }
        const names = { idle: 'inactif', connecting: 'connexion', live: 'en direct', error: 'erreur', stopped: 'arrêté' };
        const dests = Object.entries(r.status.destinations).map(([k, d]) => `${k} ${names[d.status] || d.status}`);
        rs.className = `badge ${Object.values(r.status.destinations).some((d) => d.status === 'live') ? 'on' : ''}`;
        rs.textContent = `Restream · ${r.status.obsConnected ? 'flux reçu' : 'en attente du flux'}${dests.length ? ` · ${dests.join(' · ')}` : ''}`;
        rs.title = Object.entries(r.status.destinations)
          .filter(([, d]) => d.failureReason)
          .map(([k, d]) => `${k} : ${d.failureReason}`)
          .join('\n');
      };
      pollRestream();
      setInterval(pollRestream, 4000);
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
        const rc = MS.outputs.reconnect;
        live.classList.toggle('on', Boolean(stream || rc));
        rec.classList.toggle('on', Boolean(record));
        const liveStart = stream ? stream.liveStart || stream.startedAt : rc ? rc.liveStart : 0;
        let liveText = `${rc ? 'RECONNEXION' : 'LIVE'} ${liveStart ? MS.formatTime(now - liveStart) : '00:00:00'}`;
        if (stream && stream.progress && stream.progress.bitrate) liveText += ` · ${Math.round(stream.progress.bitrate)} kb/s`;
        live.textContent = liveText;
        rec.textContent = `REC ${record ? MS.formatTime(now - record.startedAt) : '00:00:00'}`;
        perf.textContent = cpu ? `CPU ${cpu.cpu.toFixed(1)} % · ${Math.round(cpu.memMB)} Mo` : '';
        const s = MS.compositor.stats;
        const target = store().settings.video.fps;
        fps.textContent = `${s.fps.toFixed(1)} / ${target} FPS · rendu ${s.renderMs.toFixed(1)} ms · images manquées ${s.lagged}`;
      }, 500);
    },
  };

  const DOCK_LABELS = { scenes: 'Scènes', sources: 'Sources', mixer: 'Mixeur audio', transitions: 'Transitions', controls: 'Commandes', chat: 'Chat', liveinfo: 'Titre et catégorie' };

  // Graduation du mixeur, de −60 dB à 0 dB (échelle linéaire en dB, comme OBS).
  const METER_TICKS = [-60, -45, -30, -20, -10, -5, 0];
  const dbPos = (db) => MS.clamp((db + 60) / 60, 0, 1);

  function hasAudioFx(src) {
    const f = src.audioFilters;
    return Boolean(f && (f.gain || f.gate || f.compressor || f.limiter));
  }

  function gainLabel(g) {
    return g <= 0 ? '-∞ dB' : `${(20 * Math.log10(g)).toFixed(1)} dB`;
  }

  MS.ui = ui;
})();
