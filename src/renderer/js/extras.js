'use strict';

// Fonctionnalités inspirées des retours des utilisateurs d'OBS : annuler /
// rétablir, raccourcis globaux, transformation précise, sauvegarde des
// scènes et assistant de configuration pour bien démarrer.
(function () {
  const MS = window.MS;
  const el = MS.el;

  /* Annuler / rétablir ---------------------------------------------------- */

  const history = {
    undoStack: [],
    redoStack: [],
    last: null,
    restoring: false,
    LIMIT: 50,

    init() {
      this.last = this.snap();
      const schedule = MS.debounce(() => this.commit(), 400);
      const save = MS.store.save;
      // Chaque modification sauvegardée devient une étape annulable (un
      // déplacement à la souris ne compte qu'une fois, à la fin).
      MS.store.save = (...args) => {
        save(...args);
        if (!this.restoring) schedule();
      };
      this.rawSave = save;
    },

    snap() {
      const d = MS.store.data;
      return JSON.stringify({ scenes: d.scenes, sources: d.sources });
    },

    commit() {
      const cur = this.snap();
      if (cur === this.last) return;
      this.undoStack.push(this.last);
      if (this.undoStack.length > this.LIMIT) this.undoStack.shift();
      this.redoStack = [];
      this.last = cur;
    },

    undo() {
      this.commit();
      if (!this.undoStack.length) return MS.toast('Rien à annuler.', 'info', 1500);
      this.redoStack.push(this.last);
      this.apply(this.undoStack.pop());
    },

    redo() {
      if (!this.redoStack.length) return MS.toast('Rien à rétablir.', 'info', 1500);
      this.undoStack.push(this.last);
      this.apply(this.redoStack.pop());
    },

    /** Remplace scènes et sources, puis met à jour uniquement les sources qui ont changé. */
    apply(json) {
      this.restoring = true;
      const { scenes, sources } = JSON.parse(json);
      const d = MS.store.data;
      d.scenes = scenes;
      d.sources = sources;
      if (!MS.store.scene(d.currentSceneId)) d.currentSceneId = scenes[0].id;
      if (!MS.store.scene(d.previewSceneId)) d.previewSceneId = d.currentSceneId;
      for (const [id, rt] of [...MS.runtimes.map]) {
        const src = sources[id];
        if (!src) continue;
        const changed = JSON.stringify(rt.src.settings) !== JSON.stringify(src.settings);
        rt.src = src;
        if (changed) MS.runtimes.refresh(src);
      }
      MS.runtimes.sync();
      for (const id of MS.audio.channels.keys()) MS.audio.refreshFilters(id);
      MS.preview.select(null);
      this.last = json;
      this.rawSave();
      for (const evt of ['scenes:changed', 'sources:changed', 'items:changed', 'scene:switched', 'mixer:changed']) MS.emit(evt);
      this.restoring = false;
    },

    /** Remplace la collection entière (import, sauvegarde) de façon annulable. */
    replaceAll(scenes, sources) {
      this.commit();
      this.undoStack.push(this.last);
      this.redoStack = [];
      this.apply(JSON.stringify({ scenes, sources }));
    },
  };
  MS.history = history;

  /* Raccourcis clavier --------------------------------------------------- */

  const HOTKEY_LABELS = {
    toggleStream: 'Démarrer / arrêter le streaming',
    toggleRecord: "Démarrer / arrêter l'enregistrement",
    toggleMic: 'Couper / rétablir le micro',
    nextScene: 'Scène suivante',
    prevScene: 'Scène précédente',
    transition: 'Transition (mode studio)',
  };
  MS.HOTKEY_LABELS = HOTKEY_LABELS;

  const CODE_KEYS = { Space: 'Space', Enter: 'Enter', Tab: 'Tab', Escape: 'Escape', Insert: 'Insert', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Minus: '-', Equal: '=', Comma: ',', Period: '.', Slash: '/', Semicolon: ';', Quote: "'", BracketLeft: '[', BracketRight: ']', Backslash: '\\', Backquote: '`', PrintScreen: 'PrintScreen', Pause: 'MediaPlayPause' };

  /** Convertit une touche pressée en raccourci Electron (ex. « Ctrl+Shift+F9 »). */
  MS.keyToAccelerator = function (e) {
    let key = null;
    if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
    else if (/^Digit[0-9]$/.test(e.code)) key = e.code.slice(5);
    else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(e.code)) key = e.code;
    else if (/^Numpad[0-9]$/.test(e.code)) key = `num${e.code.slice(6)}`;
    else key = CODE_KEYS[e.code] || null;
    if (!key) return null;
    const mods = [];
    if (e.ctrlKey) mods.push('Ctrl');
    if (e.altKey) mods.push('Alt');
    if (e.shiftKey) mods.push('Shift');
    if (e.metaKey) mods.push('Super');
    return [...mods, key].join('+');
  };

  const hotkeys = {
    async apply() {
      const map = MS.store.settings.ui.hotkeys || {};
      const failed = await window.api.setHotkeys(map);
      if (failed.length) MS.toast(`Raccourcis déjà utilisés par une autre application : ${failed.join(', ')}`, 'error', 8000);
    },

    run(action) {
      const store = MS.store;
      const scenes = store.data.scenes;
      const step = (delta) => {
        const cur = store.editScene();
        const i = scenes.indexOf(cur);
        const next = scenes[(i + delta + scenes.length) % scenes.length];
        MS.ui.activateScene(next.id);
      };
      switch (action) {
        case 'toggleStream':
          return MS.outputs.toggle('stream');
        case 'toggleRecord':
          return MS.outputs.toggle('record');
        case 'toggleMic': {
          const mic = store.source('global_mic');
          if (!mic) return MS.toast('Aucun micro configuré.', 'error');
          mic.muted = !mic.muted;
          MS.audio.updateGains();
          store.save();
          MS.emit('mixer:changed');
          return MS.toast(mic.muted ? 'Micro coupé' : 'Micro rétabli', 'info', 1500);
        }
        case 'nextScene':
          return step(1);
        case 'prevScene':
          return step(-1);
        case 'transition':
          return store.settings.ui.studio && MS.compositor.switchScene(store.data.previewSceneId);
        default:
          return null;
      }
    },
  };
  MS.hotkeys = hotkeys;

  /* Transformation précise (Ctrl+E) ------------------------------------- */

  MS.openTransform = async function (item) {
    const r = item && MS.compositor.itemRect(item);
    if (!r) return;
    const values = { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.w), h: Math.round(r.h), keepRatio: true };
    let form;
    const onChange = (v) => {
      // Proportions conservées : la hauteur suit la largeur.
      if (v.keepRatio && v.w !== values.w) {
        const hInput = form.element.querySelectorAll('input[type="number"]')[3];
        hInput.value = Math.round((v.w * r.natH) / r.natW);
      }
      values.w = v.w;
    };
    form = MS.buildForm(
      [
        { key: 'x', type: 'number', label: 'Position X (px)', min: -10000, max: 10000 },
        { key: 'y', type: 'number', label: 'Position Y (px)', min: -10000, max: 10000 },
        { key: 'w', type: 'number', label: 'Largeur (px)', min: 1, max: 20000 },
        { key: 'h', type: 'number', label: 'Hauteur (px)', min: 1, max: 20000 },
        { key: 'keepRatio', type: 'checkbox', label: 'Conserver les proportions' },
        { type: 'info', text: `Taille d’origine : ${r.natW} × ${r.natH} px` },
      ],
      values,
      onChange
    );
    const src = MS.store.source(item.sourceId);
    const ok = await MS.modal({ title: `Transformation de « ${src ? src.name : ''} »`, content: form.element, width: 420 });
    if (!ok) return;
    const v = form.values();
    Object.assign(item, { x: v.x, y: v.y, scaleX: Math.max(1, v.w) / r.natW, scaleY: Math.max(1, v.h) / r.natH });
    MS.store.save();
    MS.emit('items:changed');
  };

  /* Filtres audio ----------------------------------------------------------- */

  MS.openAudioFilters = async function (sourceId) {
    const src = MS.store.source(sourceId);
    if (!src) return;
    const backup = src.audioFilters ? { ...src.audioFilters } : null;
    const apply = (v) => {
      src.audioFilters = { ...MS.audioFilterDefaults(), ...v };
      MS.audio.refreshFilters(src.id);
    };
    const form = MS.buildForm(
      [
        { type: 'info', text: 'Réglages appliqués en direct : parle dans le micro en regardant le VU-mètre.' },
        { key: 'gain', type: 'range', label: 'Gain (dB) : monte un micro trop faible', min: -20, max: 30 },
        { type: 'section', label: 'Noise gate (coupe le bruit de fond quand tu ne parles pas)' },
        { key: 'gate', type: 'checkbox', label: 'Activer' },
        { key: 'gateThreshold', type: 'range', label: 'Seuil (dB) : sous ce niveau, le micro est coupé', min: -80, max: 0, showIf: (v) => v.gate },
        { key: 'gateRelease', type: 'range', label: 'Fermeture (ms)', min: 20, max: 1000, showIf: (v) => v.gate },
        { type: 'section', label: 'Compresseur (voix plus régulière)' },
        { key: 'compressor', type: 'checkbox', label: 'Activer' },
        { key: 'compThreshold', type: 'range', label: 'Seuil (dB)', min: -60, max: 0, showIf: (v) => v.compressor },
        { key: 'compRatio', type: 'range', label: 'Ratio', min: 1, max: 20, showIf: (v) => v.compressor },
        { key: 'compMakeup', type: 'range', label: 'Gain de compensation (dB)', min: 0, max: 24, showIf: (v) => v.compressor },
        { type: 'section', label: 'Limiteur (empêche la saturation à 0 dB)' },
        { key: 'limiter', type: 'checkbox', label: 'Activer' },
      ],
      { ...MS.audioFilterDefaults(), ...(src.audioFilters || {}) },
      apply
    );
    const ok = await MS.modal({ title: `Filtres audio de « ${src.name} »`, content: form.element, width: 540 });
    if (ok) {
      apply(form.values());
      const isDefault = JSON.stringify(src.audioFilters) === JSON.stringify(MS.audioFilterDefaults());
      if (isDefault) delete src.audioFilters;
    } else {
      src.audioFilters = backup;
      MS.audio.refreshFilters(src.id);
    }
    MS.store.save();
    MS.emit('mixer:changed');
  };

  /* Collections de scènes ------------------------------------------------- */

  MS.exportCollection = async function () {
    const d = MS.store.data;
    const file = await window.api.exportConfig({ app: 'MyStream', version: MS.appInfo.version, scenes: d.scenes, sources: d.sources });
    if (file) MS.toast(`Collection exportée : ${file}`, 'success', 6000);
  };

  MS.importCollection = async function (fromBackups) {
    let data;
    try {
      data = await window.api.importConfig(fromBackups);
    } catch (e) {
      return MS.toast(`Fichier illisible : ${e.message}`, 'error', 8000);
    }
    if (!data) return;
    // Collection OBS choisie ici par erreur : on passe par l'import OBS.
    if (MS.isObsCollection(data)) return MS.importObsCollection(data);
    if (!Array.isArray(data.scenes) || !data.scenes.length || typeof data.sources !== 'object') {
      return MS.toast("Ce fichier n'est pas une collection de scènes MyStream.", 'error', 8000);
    }
    const ok = await MS.confirm('Remplacer les scènes', `Remplacer tes scènes actuelles par les ${data.scenes.length} scène(s) de ce fichier ? (Ctrl+Z pour revenir en arrière)`, 'Remplacer');
    if (!ok) return;
    MS.history.replaceAll(data.scenes, data.sources);
    MS.syncGlobalAudio();
    MS.toast('Collection importée.', 'success');
  };

  /* Assistant de configuration (premier lancement) ----------------------- */

  const QUALITIES = {
    '1080p60': { width: 1920, height: 1080, fps: 60, stream: 6000, label: '1080p 60 i/s (PC puissant, encodeur matériel)' },
    '1080p30': { width: 1920, height: 1080, fps: 30, stream: 4500, label: '1080p 30 i/s (équilibré)' },
    '720p60': { width: 1280, height: 720, fps: 60, stream: 4500, label: '720p 60 i/s (jeux rapides, connexion moyenne)' },
    '720p30': { width: 1280, height: 720, fps: 30, stream: 3000, label: '720p 30 i/s (PC modeste ou petite connexion)' },
  };

  MS.openWizard = async function () {
    const st = MS.store.settings;
    const encoders = MS.encoders.length ? MS.encoders : await window.api.encoders().catch(() => []);
    const hardware = encoders.some((e) => e.id !== 'x264');
    const recommended = hardware ? '1080p60' : '720p30';
    const services = Object.entries(MS.STREAM_SERVICES).map(([value, s]) => ({ value, label: s.label }));
    const mics = [{ value: 'disabled', label: 'Aucun micro' }, ...(await MS.deviceOptions('audioinput', true))];
    const values = {
      usage: 'stream',
      service: st.stream.service,
      key: st.stream.key,
      restreamUrl: st.restream.url,
      quality: recommended,
      mic: st.audio.micDevice,
      addScreen: !Object.values(MS.store.data.sources).some((s) => s.type === 'display_capture'),
    };
    const form = MS.buildForm(
      [
        { type: 'info', text: 'Quelques questions pour tout régler en une minute. Tout reste modifiable ensuite dans les Paramètres.' },
        {
          key: 'usage',
          type: 'select',
          label: 'Que veux-tu faire ?',
          options: [
            { value: 'stream', label: 'Streamer (et enregistrer)' },
            { value: 'record', label: 'Enregistrer uniquement' },
          ],
        },
        { key: 'service', type: 'select', label: 'Plateforme', options: services, showIf: (v) => v.usage === 'stream' },
        { key: 'key', type: 'password', label: 'Clé de stream', showIf: (v) => v.usage === 'stream' && v.service !== 'restream' },
        { key: 'restreamUrl', type: 'text', label: 'Adresse de Restream', showIf: (v) => v.usage === 'stream' && v.service === 'restream' },
        {
          key: 'quality',
          type: 'select',
          label: `Qualité (recommandé pour ton PC : ${recommended})`,
          options: Object.entries(QUALITIES).map(([value, q]) => ({ value, label: q.label })),
        },
        { key: 'mic', type: 'select', label: 'Micro', options: mics },
        { key: 'addScreen', type: 'checkbox', label: 'Ajouter une capture de l’écran principal à la scène' },
      ],
      values
    );
    const ok = await MS.modal({
      title: 'Bienvenue dans MyStream',
      content: form.element,
      width: 560,
      buttons: [
        { label: 'Plus tard', value: false },
        { label: 'Terminer', value: true, primary: true },
      ],
    });
    MS.store.data.wizardDone = true;
    MS.store.save();
    if (!ok) return;

    const v = form.values();
    const q = QUALITIES[v.quality];
    if (v.usage === 'stream') {
      st.stream.service = v.service;
      if (v.service === 'restream') st.restream.url = v.restreamUrl;
      else {
        st.stream.key = v.key;
        if (MS.STREAM_SERVICES[v.service].server) st.stream.server = MS.STREAM_SERVICES[v.service].server;
      }
    }
    st.stream.videoBitrate = q.stream;
    st.stream.encoder = 'auto';
    st.audio.micDevice = v.mic;
    if (!MS.outputs.isAnyActive() && (st.video.width !== q.width || st.video.height !== q.height || st.video.fps !== q.fps)) {
      Object.assign(st.video, { width: q.width, height: q.height, fps: q.fps });
      MS.outputs.resetStream();
      MS.compositor.applyVideoSettings();
    }
    if (v.addScreen) {
      const src = MS.store.createSource('display_capture', 'Écran', { captureId: '', captureName: '', captureKind: 'screen' });
      const item = MS.store.addItem(MS.store.editScene().id, src.id);
      // L'écran occupera tout le canevas dès que sa taille sera connue.
      const fit = setInterval(() => {
        const r = MS.compositor.itemRect(item);
        if (!r) return;
        clearInterval(fit);
        MS.preview.transform(item, 'fit');
      }, 300);
      setTimeout(() => clearInterval(fit), 15000);
    }
    MS.store.save();
    MS.syncGlobalAudio();
    MS.emit('settings:changed');
    MS.toast('Configuration terminée. Bon stream !', 'success', 5000);
  };
})();
