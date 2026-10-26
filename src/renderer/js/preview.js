'use strict';

(function () {
  const MS = window.MS;

  const HANDLE = 6; // demi-taille des poignées, en pixels écran
  const SNAP = 12; // distance d'aimantation, en pixels écran
  const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
  const CURSORS = { nw: 'nwse-resize', se: 'nwse-resize', ne: 'nesw-resize', sw: 'nesw-resize', n: 'ns-resize', s: 'ns-resize', e: 'ew-resize', w: 'ew-resize' };

  const preview = {
    selectedId: null,
    drag: null,

    init(area, overlay) {
      this.area = area;
      this.programWrap = MS.$('#preview-wrap');
      this.studioWrap = MS.$('#studio-wrap');
      this.studioCanvas = MS.$('#studio-canvas');
      this.overlay = overlay;
      this.octx = overlay.getContext('2d');
      this.applyStudio();

      new ResizeObserver(() => this.layout()).observe(area);
      MS.on('video:resized', () => this.layout());
      MS.on('scene:switched', () => !MS.store.settings.ui.studio && this.select(null));
      MS.on('preview:switched', () => this.select(null));
      MS.on('studio:changed', () => {
        this.select(null);
        this.applyStudio();
      });

      overlay.addEventListener('pointerdown', (e) => this.onDown(e));
      overlay.addEventListener('pointermove', (e) => this.onMove(e));
      overlay.addEventListener('pointerup', (e) => this.onUp(e));
      overlay.addEventListener('contextmenu', (e) => this.onContext(e));
      window.addEventListener('keydown', (e) => this.onKey(e));

      const loop = () => {
        this.drawOverlay();
        requestAnimationFrame(loop);
      };
      requestAnimationFrame(loop);
    },

    /** En mode studio, l'édition se fait sur l'aperçu (à gauche) et non sur le programme. */
    applyStudio() {
      const studio = MS.store.settings.ui.studio;
      this.area.classList.toggle('studio', studio);
      this.wrap = studio ? this.studioWrap : this.programWrap;
      this.wrap.append(this.overlay);
      this.layout();
    },

    layout() {
      const { width, height } = MS.store.settings.video;
      const studio = MS.store.settings.ui.studio;
      const aw = (studio ? (this.area.clientWidth - 140) / 2 : this.area.clientWidth) - 16;
      const ah = this.area.clientHeight - (studio ? 40 : 16);
      const scale = Math.max(0.01, Math.min(aw / width, ah / height));
      const w = Math.floor(width * scale);
      const h = Math.floor(height * scale);
      const dpr = window.devicePixelRatio || 1;
      for (const wrap of [this.programWrap, this.studioWrap]) {
        wrap.style.width = `${w}px`;
        wrap.style.height = `${h}px`;
      }
      this.overlay.width = Math.round(w * dpr);
      this.overlay.height = Math.round(h * dpr);
      this.studioCanvas.width = studio ? Math.round(w * dpr) : 1;
      this.studioCanvas.height = studio ? Math.round(h * dpr) : 1;
      this.lastOverlay = null;
    },

    /** Échelle scène -> pixels CSS de l'aperçu. */
    get scale() {
      return this.wrap.clientWidth / MS.store.settings.video.width;
    },

    toScene(e) {
      const r = this.overlay.getBoundingClientRect();
      const s = this.scale;
      return { x: (e.clientX - r.left) / s, y: (e.clientY - r.top) / s };
    },

    select(id) {
      if (this.selectedId === id) return;
      this.selectedId = id;
      MS.emit('selection:changed', id);
    },

    selectedItem() {
      const scene = MS.store.editScene();
      return scene && this.selectedId ? scene.items.find((it) => it.id === this.selectedId) || null : null;
    },

    handlePoints(r) {
      const cx = r.x + r.w / 2;
      const cy = r.y + r.h / 2;
      return {
        nw: [r.x, r.y], n: [cx, r.y], ne: [r.x + r.w, r.y], e: [r.x + r.w, cy],
        se: [r.x + r.w, r.y + r.h], s: [cx, r.y + r.h], sw: [r.x, r.y + r.h], w: [r.x, cy],
      };
    },

    hitHandle(p) {
      const item = this.selectedItem();
      if (!item || item.locked) return null;
      const r = MS.compositor.itemRect(item);
      if (!r) return null;
      const tol = (HANDLE + 3) / this.scale;
      const pts = this.handlePoints(r);
      return HANDLES.find((h) => Math.abs(p.x - pts[h][0]) <= tol && Math.abs(p.y - pts[h][1]) <= tol) || null;
    },

    hitItem(p) {
      const scene = MS.store.editScene();
      if (!scene) return null;
      for (const item of scene.items) {
        if (!item.visible) continue;
        const r = MS.compositor.itemRect(item);
        if (r && p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h) return item;
      }
      return null;
    },

    onDown(e) {
      if (e.button !== 0) return;
      const p = this.toScene(e);
      const handle = this.hitHandle(p);
      let item = this.selectedItem();
      if (!handle) {
        item = this.hitItem(p);
        this.select(item ? item.id : null);
      }
      if (!item || item.locked) return;
      const r = MS.compositor.itemRect(item);
      if (!r) return;
      this.overlay.setPointerCapture(e.pointerId);
      this.drag = { mode: handle ? 'resize' : 'move', handle, start: p, rect: r, item };
    },

    onMove(e) {
      const p = this.toScene(e);
      if (!this.drag) {
        const h = this.hitHandle(p);
        this.overlay.style.cursor = h ? CURSORS[h] : this.hitItem(p) ? 'move' : 'default';
        return;
      }
      const { item, rect, start } = this.drag;
      const dx = p.x - start.x;
      const dy = p.y - start.y;
      const { width: W, height: H } = MS.store.settings.video;
      const snap = SNAP / this.scale;

      if (this.drag.mode === 'move') {
        let x = rect.x + dx;
        let y = rect.y + dy;
        if (!e.altKey) {
          x = snapTo(x, rect.w, W, snap);
          y = snapTo(y, rect.h, H, snap);
        }
        item.x = Math.round(x);
        item.y = Math.round(y);
      } else {
        const h = this.drag.handle;
        const hx = h.includes('w') ? -1 : h.includes('e') ? 1 : 0;
        const hy = h.includes('n') ? -1 : h.includes('s') ? 1 : 0;
        let w = Math.max(4, rect.w + hx * dx);
        let hh = Math.max(4, rect.h + hy * dy);
        if (hx && hy && !e.shiftKey) {
          const k = Math.max(w / rect.w, hh / rect.h);
          w = rect.w * k;
          hh = rect.h * k;
        }
        item.x = Math.round(hx === -1 ? rect.x + rect.w - w : rect.x);
        item.y = Math.round(hy === -1 ? rect.y + rect.h - hh : rect.y);
        item.scaleX = w / rect.natW;
        item.scaleY = hh / rect.natH;
      }
    },

    onUp() {
      if (!this.drag) return;
      this.drag = null;
      MS.store.save();
    },

    onContext(e) {
      e.preventDefault();
      const item = this.hitItem(this.toScene(e));
      this.select(item ? item.id : null);
      if (item) MS.ui.itemContextMenu(item, e.clientX, e.clientY);
    },

    onKey(e) {
      const tag = document.activeElement && document.activeElement.tagName;
      if (['INPUT', 'TEXTAREA', 'SELECT'].includes(tag) || document.querySelector('.modal-backdrop')) return;
      const ctrl = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      if (ctrl && k === 'z' && !e.shiftKey) {
        e.preventDefault();
        return MS.history.undo();
      }
      if (ctrl && (k === 'y' || (k === 'z' && e.shiftKey))) {
        e.preventDefault();
        return MS.history.redo();
      }
      const item = this.selectedItem();
      if (ctrl && k === 'e' && item) {
        e.preventDefault();
        return MS.openTransform(item);
      }
      if (!item) return;
      const step = e.shiftKey ? 10 : 1;
      const moves = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
      if (moves[e.key] && !item.locked) {
        e.preventDefault();
        item.x += moves[e.key][0];
        item.y += moves[e.key][1];
        MS.store.save();
      } else if (e.key === 'Delete') {
        e.preventDefault();
        MS.ui.removeItem(item);
      }
    },

    /* Transformations rapides (menu contextuel) ------------------------- */
    transform(item, kind) {
      const r = MS.compositor.itemRect(item);
      if (!r) return;
      const { width: W, height: H } = MS.store.settings.video;
      if (kind === 'fit') {
        const s = Math.min(W / r.natW, H / r.natH);
        Object.assign(item, { scaleX: s, scaleY: s, x: Math.round((W - r.natW * s) / 2), y: Math.round((H - r.natH * s) / 2) });
      } else if (kind === 'stretch') {
        Object.assign(item, { scaleX: W / r.natW, scaleY: H / r.natH, x: 0, y: 0 });
      } else if (kind === 'center') {
        Object.assign(item, { x: Math.round((W - r.w) / 2), y: Math.round((H - r.h) / 2) });
      } else if (kind === 'reset') {
        Object.assign(item, { scaleX: 1, scaleY: 1, x: 0, y: 0 });
      }
      MS.store.save();
    },

    drawOverlay() {
      const item = this.selectedItem();
      const r = item && item.visible ? MS.compositor.itemRect(item) : null;
      // On ne redessine que si la sélection a changé à l'écran.
      const sig = r ? `${item.id}|${r.x}|${r.y}|${r.w}|${r.h}|${item.locked}` : '';
      if (sig === this.lastOverlay) return;
      this.lastOverlay = sig;
      const ctx = this.octx;
      const dpr = window.devicePixelRatio || 1;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.clearRect(0, 0, this.overlay.width, this.overlay.height);
      if (!r) return;
      const s = this.scale * dpr;
      ctx.setTransform(s, 0, 0, s, 0, 0);
      const px = 1 / s;
      ctx.lineWidth = 2 * dpr * px;
      ctx.strokeStyle = item.locked ? '#9aa0b4' : '#ff4d5a';
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      if (item.locked) return;
      const hs = HANDLE * dpr * px;
      ctx.fillStyle = '#ff4d5a';
      for (const [x, y] of Object.values(this.handlePoints(r))) ctx.fillRect(x - hs, y - hs, hs * 2, hs * 2);
    },
  };

  function snapTo(pos, size, max, dist) {
    if (Math.abs(pos) < dist) return 0;
    if (Math.abs(pos + size - max) < dist) return max - size;
    if (Math.abs(pos + size / 2 - max / 2) < dist) return (max - size) / 2;
    return pos;
  }

  MS.preview = preview;
})();
