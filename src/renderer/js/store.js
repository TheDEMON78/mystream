'use strict';

(function () {
  const MS = window.MS;

  const STREAM_SERVICES = {
    twitch: { label: 'Twitch', server: 'rtmp://live.twitch.tv/app' },
    youtube: { label: 'YouTube', server: 'rtmp://a.rtmp.youtube.com/live2' },
    facebook: { label: 'Facebook Live', server: 'rtmps://live-api-s.facebook.com:443/rtmp' },
    custom: { label: 'Personnalisé', server: '' },
  };
  MS.STREAM_SERVICES = STREAM_SERVICES;

  function defaults() {
    return {
      version: 1,
      settings: {
        video: { width: 1920, height: 1080, fps: 30 },
        stream: { service: 'twitch', server: STREAM_SERVICES.twitch.server, key: '', videoBitrate: 4500, audioBitrate: 160, preset: 'veryfast' },
        output: { recordPath: '', format: 'mp4', videoBitrate: 8000, audioBitrate: 192, preset: 'veryfast' },
        audio: { micDevice: 'default', desktopAudio: false },
        transition: { type: 'fade', duration: 300 },
      },
      sources: {},
      scenes: [],
      currentSceneId: null,
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
      if (!this.data.scenes.length) this.addScene('Scène', true);
      if (!this.scene(this.data.currentSceneId)) this.data.currentSceneId = this.data.scenes[0].id;
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
      this.save();
      MS.emit('scenes:changed');
      MS.emit('sources:changed');
      return true;
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
