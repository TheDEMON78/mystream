'use strict';

(function () {
  const MS = window.MS;

  const STREAM_SERVICES = {
    restream: { label: 'Restream (BloopStudio)', server: '' },
    twitch: { label: 'Twitch', server: 'rtmp://live.twitch.tv/app' },
    youtube: { label: 'YouTube', server: 'rtmp://a.rtmp.youtube.com/live2' },
    kick: { label: 'Kick', server: 'rtmps://fa723fc1b171.global-contribute.live-video.net:443/app' },
    facebook: { label: 'Facebook Live', server: 'rtmps://live-api-s.facebook.com:443/rtmp' },
    custom: { label: 'Personnalisé', server: '' },
  };
  MS.STREAM_SERVICES = STREAM_SERVICES;

  function defaults() {
    return {
      version: 1,
      settings: {
        video: { width: 1920, height: 1080, fps: 60 },
        // Qualité maximale par défaut : 6000 kb/s et 160 kb/s sont les plafonds de Twitch.
        stream: { service: 'twitch', server: STREAM_SERVICES.twitch.server, key: '', videoBitrate: 6000, audioBitrate: 160, preset: 'veryfast', encoder: 'auto' },
        output: { recordPath: '', format: 'mp4', videoBitrate: 40000, audioBitrate: 320, preset: 'veryfast' },
        audio: { micDevice: 'default', desktopAudio: false },
        transition: { type: 'fade', duration: 300, direction: 'left', stingerFile: '', stingerPoint: 600, stingerAudio: true },
        liveInfo: { titles: [] },
        restream: { url: 'http://localhost:3050', password: '', allowSelfSigned: true, rtmpUrl: '' },
        chat: { mode: 'off', twitchChannel: '', kickChannel: '', kickChatroomId: '' },
        // Télécommande compatible obs-websocket (Stream Deck…) ; mot de passe créé au premier lancement.
        remote: { enabled: true, port: 4455, password: null, lan: false },
        ui: {
          studio: false,
          docks: { scenes: true, sources: true, mixer: true, transitions: true, controls: true, chat: true, liveinfo: true },
          liveBar: true,
          preview: true, // aperçu affiché
          browserGpu: true,
          gpuCapture: true, // capture de jeu sur la carte graphique (Windows Graphics Capture) // sources Navigateur rendues directement sur la carte graphique
          autoHidePreview: true, // masquer l'aperçu quand MyStream n'est pas au premier plan pendant un live
          layout: null, // disposition des docks (null = disposition d'origine)
          layouts: {}, // dispositions enregistrées par nom
          minimizeToTray: false,
          confirmStopStream: true,
          hotkeys: { toggleStream: '', toggleRecord: '', toggleMic: '', nextScene: '', prevScene: '', transition: '' },
        },
      },
      sources: {},
      scenes: [],
      currentSceneId: null,
      previewSceneId: null,
      liveHistory: [],
    };
  }

  function deepMerge(base, over) {
    if (!over || typeof over !== 'object' || Array.isArray(over)) return over === undefined ? base : over;
    const out = { ...base };
    for (const [k, v] of Object.entries(over)) {
      out[k] = base && typeof base[k] === 'object' && !Array.isArray(base[k]) ? deepMerge(base[k], v) : v;
    }
    return out;
  }

  MS.defaultSettings = () => defaults().settings;

  const store = {
    data: null,

    async load() {
      let saved = null;
      try {
        saved = await window.api.loadConfig();
      } catch (e) {
        console.warn('Configuration illisible', e);
      }
      const d = defaults();
      this.data = saved ? { ...d, ...saved, settings: deepMerge(d.settings, saved.settings) } : d;
      this.fresh = !saved; // premier lancement : l'assistant de configuration s'ouvrira
      if (this.data.settings.remote.password == null) {
        this.data.settings.remote.password = MS.remote.newPassword();
        this.save();
      }
      // Mise à niveau unique : aperçu masqué en jeu pendant un live (allège la carte graphique).
      const done = (this.data.migrations = this.data.migrations || {});
      if (!done.autoHidePreview) {
        done.autoHidePreview = true;
        this.data.settings.ui.autoHidePreview = true;
        this.save();
      }
      if (!this.data.scenes.length) this.addScene('Scène', true);
      if (!this.scene(this.data.currentSceneId)) this.data.currentSceneId = this.data.scenes[0].id;
      if (!this.scene(this.data.previewSceneId)) this.data.previewSceneId = this.data.currentSceneId;
    },

    save: MS.debounce(() => {
      window.api.saveConfig(store.data).catch((e) => console.error('Sauvegarde impossible', e));
    }, 400),

    get settings() {
      return this.data.settings;
    },

    /* Scènes ---------------------------------------------------------- */
    scene(id) {
      return this.data.scenes.find((s) => s.id === id) || null;
    },

    currentScene() {
      return this.scene(this.data.currentSceneId);
    },

    /** Scène en cours d'édition : l'aperçu en mode studio, sinon le programme. */
    editScene() {
      return (this.settings.ui.studio && this.scene(this.data.previewSceneId)) || this.currentScene();
    },

    setPreviewScene(id) {
      if (!this.scene(id) || this.data.previewSceneId === id) return;
      this.data.previewSceneId = id;
      this.save();
      MS.emit('preview:switched', id);
    },

    setStudio(on) {
      this.settings.ui.studio = Boolean(on);
      if (on) this.data.previewSceneId = this.data.currentSceneId;
      this.save();
      MS.emit('studio:changed', this.settings.ui.studio);
    },

    addScene(name, silent) {
      const scene = { id: MS.uid(), name: this.uniqueName(name, this.data.scenes.map((s) => s.name)), items: [] };
      this.data.scenes.push(scene);
      if (!silent) {
        this.save();
        MS.emit('scenes:changed');
      }
      return scene;
    },

    renameScene(id, name) {
      const scene = this.scene(id);
      if (!scene || !name) return;
      scene.name = name;
      this.save();
      MS.emit('scenes:changed');
    },

    removeScene(id) {
      if (this.data.scenes.length <= 1) return false;
      const scene = this.scene(id);
      if (!scene) return false;
      for (const item of [...scene.items]) this.removeItem(scene.id, item.id, true);
      this.data.scenes = this.data.scenes.filter((s) => s.id !== id);
      if (this.data.currentSceneId === id) this.setCurrentScene(this.data.scenes[0].id);
      if (this.data.previewSceneId === id) this.setPreviewScene(this.data.currentSceneId);
      this.save();
      MS.emit('scenes:changed');
      MS.emit('sources:changed');
      return true;
    },

    /**
     * Copie d'une scène, positions indépendantes. Sources partagées par défaut
     * (comme dans OBS) ; avec « independent », chaque source est copiée.
     */
    duplicateScene(id, independent) {
      const scene = this.scene(id);
      if (!scene) return null;
      const name = this.uniqueName(`${scene.name} (copie)`, this.data.scenes.map((s) => s.name));
      const copy = {
        id: MS.uid(),
        name,
        items: scene.items.map((it) => ({ ...it, id: MS.uid(), sourceId: independent ? (this.cloneSource(it.sourceId, name) || { id: it.sourceId }).id : it.sourceId })),
      };
      if (independent) MS.emit('sources:changed');
      this.data.scenes.splice(this.data.scenes.indexOf(scene) + 1, 0, copy);
      this.save();
      MS.emit('scenes:changed');
      return copy;
    },

    moveScene(id, delta) {
      const list = this.data.scenes;
      const i = list.findIndex((s) => s.id === id);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= list.length) return;
      [list[i], list[j]] = [list[j], list[i]];
      this.save();
      MS.emit('scenes:changed');
    },

    setCurrentScene(id) {
      if (!this.scene(id) || this.data.currentSceneId === id) return;
      this.data.currentSceneId = id;
      this.save();
      MS.emit('scene:switched', id);
    },

    /* Sources --------------------------------------------------------- */
    source(id) {
      return this.data.sources[id] || null;
    },

    createSource(type, name, settings, extra = {}) {
      const src = {
        id: extra.id || MS.uid(),
        type,
        name: this.uniqueName(name, Object.values(this.data.sources).map((s) => s.name)),
        settings,
        volume: 1,
        muted: false,
        monitor: false,
        ...extra,
      };
      this.data.sources[src.id] = src;
      this.save();
      MS.emit('sources:changed');
      return src;
    },

    updateSource(id, patch) {
      const src = this.source(id);
      if (!src) return;
      Object.assign(src, patch);
      this.save();
      MS.emit('sources:changed');
    },

    deleteSource(id) {
      if (!this.data.sources[id]) return;
      delete this.data.sources[id];
      for (const scene of this.data.scenes) scene.items = scene.items.filter((it) => it.sourceId !== id);
      this.save();
      MS.emit('sources:changed');
      MS.emit('items:changed');
    },

    /** Scènes où la source apparaît. */
    scenesUsing(id) {
      return this.data.scenes.filter((s) => s.items.some((it) => it.sourceId === id));
    },

    /** Copie indépendante d'une source (mêmes réglages et filtres, nouvel identifiant). */
    cloneSource(id, suffix) {
      const src = this.source(id);
      if (!src) return null;
      const copy = JSON.parse(JSON.stringify(src));
      copy.id = MS.uid();
      copy.name = this.uniqueName(suffix ? `${src.name} (${suffix})` : src.name, Object.values(this.data.sources).map((s) => s.name));
      delete copy.global;
      this.data.sources[copy.id] = copy;
      return copy;
    },

    /**
     * Source partagée entre plusieurs scènes, comme dans OBS : la modifier
     * dans une scène la modifie partout. Ici, l'élément de cette scène reçoit
     * sa propre copie, réglable à part.
     */
    unlinkItem(sceneId, itemId) {
      const item = this.item(sceneId, itemId);
      const scene = this.scene(sceneId);
      if (!item || !scene) return null;
      const copy = this.cloneSource(item.sourceId, scene.name);
      if (!copy) return null;
      item.sourceId = copy.id;
      this.save();
      MS.emit('sources:changed');
      MS.emit('items:changed');
      return copy;
    },

    /** L'élément utilise désormais une source existante (partagée) ; l'ancienne disparaît si plus rien ne l'utilise. */
    linkItem(sceneId, itemId, sourceId) {
      const item = this.item(sceneId, itemId);
      if (!item || !this.source(sourceId) || item.sourceId === sourceId) return;
      const old = this.source(item.sourceId);
      item.sourceId = sourceId;
      if (old && !old.global && !this.isSourceUsed(old.id)) delete this.data.sources[old.id];
      this.save();
      MS.emit('sources:changed');
      MS.emit('items:changed');
    },

    isSourceUsed(id) {
      return this.data.scenes.some((s) => s.items.some((it) => it.sourceId === id));
    },

    /* Éléments de scène ------------------------------------------------ */
    item(sceneId, itemId) {
      const scene = this.scene(sceneId);
      return scene ? scene.items.find((it) => it.id === itemId) || null : null;
    },

    addItem(sceneId, sourceId) {
      const scene = this.scene(sceneId);
      if (!scene) return null;
      const item = { id: MS.uid(), sourceId, x: 0, y: 0, scaleX: null, scaleY: null, visible: true, locked: false };
      scene.items.unshift(item);
      this.save();
      MS.emit('items:changed');
      return item;
    },

    updateItem(sceneId, itemId, patch, silent) {
      const item = this.item(sceneId, itemId);
      if (!item) return;
      Object.assign(item, patch);
      this.save();
      if (!silent) MS.emit('items:changed');
    },

    removeItem(sceneId, itemId, silent) {
      const scene = this.scene(sceneId);
      if (!scene) return;
      const item = scene.items.find((it) => it.id === itemId);
      if (!item) return;
      scene.items = scene.items.filter((it) => it.id !== itemId);
      const src = this.source(item.sourceId);
      if (src && !src.global && !this.isSourceUsed(src.id)) delete this.data.sources[src.id];
      this.save();
      if (!silent) {
        MS.emit('items:changed');
        MS.emit('sources:changed');
      }
    },

    moveItem(sceneId, itemId, delta) {
      const scene = this.scene(sceneId);
      if (!scene) return;
      const list = scene.items;
      const i = list.findIndex((it) => it.id === itemId);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= list.length) return;
      [list[i], list[j]] = [list[j], list[i]];
      this.save();
      MS.emit('items:changed');
    },

    /* Divers ----------------------------------------------------------- */
    uniqueName(base, names) {
      const set = new Set(names);
      if (!set.has(base)) return base;
      let i = 2;
      while (set.has(`${base} ${i}`)) i++;
      return `${base} ${i}`;
    },
  };

  MS.store = store;
})();
