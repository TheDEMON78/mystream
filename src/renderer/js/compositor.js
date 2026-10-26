'use strict';

(function () {
  const MS = window.MS;

  const compositor = {
    canvas: null,
    ctx: null,
    timer: null,
    transition: null,
    stats: { fps: 0, renderMs: 0, lagged: 0, total: 0 },
    _frames: 0,
    _statsAt: 0,
    _last: 0,

    init(canvas) {
      this.canvas = canvas;
      this.ctx = canvas.getContext('2d', { alpha: false });
      this.applyVideoSettings();
    },

    applyVideoSettings() {
      const v = MS.store.settings.video;
      this.canvas.width = v.width;
      this.canvas.height = v.height;
      clearInterval(this.timer);
      this.interval = 1000 / v.fps;
      this._last = performance.now();
      this.timer = setInterval(() => this.frame(), this.interval);
      MS.emit('video:resized');
    },

    switchScene(id) {
      const from = MS.store.data.currentSceneId;
      if (from === id) return;
      const t = MS.store.settings.transition;
      if (t.type === 'fade' && t.duration > 0 && from) {
        this.transition = { from, start: performance.now(), duration: Number(t.duration) };
      } else {
        this.transition = null;
      }
      MS.store.setCurrentScene(id);
    },

    /** Taille affichée d'un élément, ou null tant que la source n'a pas de dimensions. */
    itemRect(item) {
      const rt = MS.runtimes.get(item.sourceId);
      if (!rt || !rt.width || !rt.height) return null;
      if (item.scaleX == null) this.initItem(item, rt);
      return { x: item.x, y: item.y, w: rt.width * item.scaleX, h: rt.height * item.scaleY, natW: rt.width, natH: rt.height };
    },

    /** Première apparition : comme OBS, on place l'élément en haut à gauche et on le réduit s'il dépasse. */
    initItem(item, rt) {
      const { width, height } = MS.store.settings.video;
      const s = Math.min(1, width / rt.width, height / rt.height);
      item.scaleX = s;
      item.scaleY = s;
      MS.store.save();
      MS.emit('items:changed');
    },

    drawScene(scene, alpha) {
      if (!scene) return;
      const ctx = this.ctx;
      ctx.globalAlpha = alpha;
      for (let i = scene.items.length - 1; i >= 0; i--) {
        const item = scene.items[i];
        if (!item.visible) continue;
        const rt = MS.runtimes.get(item.sourceId);
        const d = rt && rt.drawable();
        if (!d) continue;
        const r = this.itemRect(item);
        if (!r || r.w <= 0 || r.h <= 0) continue;
        ctx.drawImage(d, r.x, r.y, r.w, r.h);
      }
      ctx.globalAlpha = 1;
    },

    frame() {
      const t0 = performance.now();
      if (t0 - this._last > this.interval * 1.5) this.stats.lagged++;
      this._last = t0;
      this.stats.total++;

      const ctx = this.ctx;
      ctx.globalAlpha = 1;
      ctx.fillStyle = '#000';
      ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);

      const current = MS.store.currentScene();
      if (this.transition) {
        const p = (t0 - this.transition.start) / this.transition.duration;
        if (p >= 1) {
          this.transition = null;
          this.drawScene(current, 1);
        } else {
          this.drawScene(MS.store.scene(this.transition.from), 1);
          this.drawScene(current, p);
        }
      } else {
        this.drawScene(current, 1);
      }

      const t1 = performance.now();
      this.stats.renderMs = this.stats.renderMs * 0.9 + (t1 - t0) * 0.1;
      this._frames++;
      if (t1 - this._statsAt >= 1000) {
        this.stats.fps = (this._frames * 1000) / (t1 - this._statsAt);
        this._frames = 0;
        this._statsAt = t1;
      }
    },
  };

  MS.compositor = compositor;
})();
