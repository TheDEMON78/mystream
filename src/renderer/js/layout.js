'use strict';

(function () {
  const MS = window.MS;
  const el = MS.el;

  // Deux zones de docks : la rangée du bas et la colonne à droite de l'aperçu.
  const DOCK_IDS = ['scenes', 'sources', 'mixer', 'transitions', 'controls', 'chat', 'liveinfo'];
  const MIN_DOCK = 90; // px
  const DRAG_THRESHOLD = 6;

  function defaultLayout() {
    return {
      bottom: ['scenes', 'sources', 'mixer', 'transitions', 'controls'],
      side: ['chat', 'liveinfo'],
      weights: { scenes: 1, sources: 1.3, mixer: 2, transitions: 0.9, controls: 1, chat: 1.7, liveinfo: 1 },
      bottomHeight: 290,
      sideWidth: 340,
    };
  }

  /** Disposition valide : chaque dock présent une seule fois, dans une seule zone. */
  function normalize(l) {
    const d = defaultLayout();
    const out = {
      bottom: Array.isArray(l && l.bottom) ? l.bottom.filter((id) => DOCK_IDS.includes(id)) : d.bottom,
      side: Array.isArray(l && l.side) ? l.side.filter((id) => DOCK_IDS.includes(id)) : d.side,
      weights: { ...d.weights, ...((l && l.weights) || {}) },
      bottomHeight: Number(l && l.bottomHeight) || d.bottomHeight,
      sideWidth: Number(l && l.sideWidth) || d.sideWidth,
    };
    out.side = out.side.filter((id) => !out.bottom.includes(id));
    for (const id of DOCK_IDS) {
      if (out.bottom.includes(id) || out.side.includes(id)) continue;
      (d.side.includes(id) ? out.side : out.bottom).push(id);
    }
    return out;
  }

  const layout = {
    init() {
      this.zones = { bottom: MS.$('#docks'), side: MS.$('#side-docks') };
      // Références gardées : un dock peut être momentanément hors de la page pendant qu'on le déplace.
      this.docks = Object.fromEntries(DOCK_IDS.map((id) => [id, MS.$(`#dock-${id}`)]));
      this.bindOuterSplitter(MS.$('#hsplit'), 'y');
      this.bindOuterSplitter(MS.$('#vsplit'), 'x');
      for (const id of DOCK_IDS) this.bindDrag(id);
      window.addEventListener('resize', () => this.clampSizes());
      this.apply();
    },

    get current() {
      const ui = MS.store.settings.ui;
      if (!ui.layout) ui.layout = defaultLayout();
      return ui.layout;
    },

    apply() {
      const ui = MS.store.settings.ui;
      ui.layout = normalize(ui.layout);
      const l = ui.layout;
      const docks = ui.docks;
      const visible = {};
      for (const zone of ['bottom', 'side']) {
        const z = this.zones[zone];
        const shown = l[zone].filter((id) => docks[id]);
        visible[zone] = shown.length;
        const children = [];
        shown.forEach((id, i) => {
          if (i > 0) children.push(this.innerSplitter(zone, shown[i - 1], id));
          const d = this.docks[id];
          d.hidden = false;
          d.style.flex = `${l.weights[id] || 1} 1 0`;
          children.push(d);
        });
        // Les docks masqués restent dans la page (leur contenu continue d'exister).
        for (const id of l[zone].filter((x) => !docks[x])) {
          const d = this.docks[id];
          d.hidden = true;
          children.push(d);
        }
        z.replaceChildren(...children);
        z.classList.toggle('empty', !shown.length);
      }
      const app = MS.$('#app');
      const hasBottom = visible.bottom > 0;
      const hasSide = visible.side > 0;
      this.clampSizes(false);
      app.style.gridTemplateRows = `auto auto minmax(0, 1fr) ${hasBottom ? '5px' : '0'} ${hasBottom ? `${l.bottomHeight}px` : '0'} auto`;
      MS.$('#hsplit').hidden = !hasBottom;
      this.zones.bottom.hidden = !hasBottom;
      MS.$('#main').style.gridTemplateColumns = hasSide ? `minmax(0, 1fr) 5px ${l.sideWidth}px` : 'minmax(0, 1fr)';
      MS.$('#vsplit').hidden = !hasSide;
      this.zones.side.hidden = !hasSide;
      MS.emit('layout:changed');
    },

    /** Garde la rangée du bas et la colonne de droite dans des tailles raisonnables. */
    clampSizes(reapply = true) {
      const l = this.current;
      const bh = MS.clamp(l.bottomHeight, 120, Math.max(120, window.innerHeight - 260));
      const sw = MS.clamp(l.sideWidth, 200, Math.max(200, window.innerWidth - 420));
      if (bh === l.bottomHeight && sw === l.sideWidth) return;
      l.bottomHeight = bh;
      l.sideWidth = sw;
      if (reapply) this.apply();
    },

    /* Redimensionnement ------------------------------------------------- */
    bindOuterSplitter(handle, axis) {
      handle.addEventListener('pointerdown', (e) => {
        e.preventDefault();
        handle.setPointerCapture(e.pointerId);
        const l = this.current;
        const start = axis === 'y' ? e.clientY : e.clientX;
        const startSize = axis === 'y' ? l.bottomHeight : l.sideWidth;
        handle.classList.add('active');
        const move = (ev) => {
          const delta = (axis === 'y' ? ev.clientY : ev.clientX) - start;
          if (axis === 'y') {
            l.bottomHeight = MS.clamp(startSize - delta, 120, window.innerHeight - 260);
            MS.$('#app').style.gridTemplateRows = `auto auto minmax(0, 1fr) 5px ${l.bottomHeight}px auto`;
          } else {
            l.sideWidth = MS.clamp(startSize - delta, 200, window.innerWidth - 420);
            MS.$('#main').style.gridTemplateColumns = `minmax(0, 1fr) 5px ${l.sideWidth}px`;
          }
          MS.emit('layout:changed');
        };
        const up = () => {
          handle.classList.remove('active');
          handle.removeEventListener('pointermove', move);
          handle.removeEventListener('pointerup', up);
          MS.store.save();
        };
        handle.addEventListener('pointermove', move);
        handle.addEventListener('pointerup', up);
      });
    },

    innerSplitter(zone, a, b) {
      const axis = zone === 'bottom' ? 'x' : 'y';
      const handle = el('div', { class: `dock-split ${axis}`, title: 'Glisser pour redimensionner' });
      handle.addEventListener('pointerdown', (e) => {
        e.preventDefault();
        handle.setPointerCapture(e.pointerId);
        const l = this.current;
        const da = this.docks[a];
        const db = this.docks[b];
        const size = (d) => (axis === 'x' ? d.getBoundingClientRect().width : d.getBoundingClientRect().height);
        const pa = size(da);
        const pb = size(db);
        const wsum = (l.weights[a] || 1) + (l.weights[b] || 1);
        const start = axis === 'x' ? e.clientX : e.clientY;
        handle.classList.add('active');
        const move = (ev) => {
          const delta = (axis === 'x' ? ev.clientX : ev.clientY) - start;
          const na = MS.clamp(pa + delta, MIN_DOCK, pa + pb - MIN_DOCK);
          l.weights[a] = (wsum * na) / (pa + pb);
          l.weights[b] = wsum - l.weights[a];
          da.style.flex = `${l.weights[a]} 1 0`;
          db.style.flex = `${l.weights[b]} 1 0`;
        };
        const up = () => {
          handle.classList.remove('active');
          handle.removeEventListener('pointermove', move);
          handle.removeEventListener('pointerup', up);
          MS.store.save();
        };
        handle.addEventListener('pointermove', move);
        handle.addEventListener('pointerup', up);
      });
      return handle;
    },

    /* Déplacement d'un dock par son titre ----------------------------- */
    bindDrag(id) {
      const dock = this.docks[id];
      const title = dock.querySelector('.dock-title');
      title.classList.add('draggable');
      title.title = 'Glisser pour déplacer ce dock';
      title.addEventListener('pointerdown', (e) => {
        if (e.button !== 0 || e.target.closest('button, input, select')) return;
        const sx = e.clientX;
        const sy = e.clientY;
        let drag = null;
        const move = (ev) => {
          if (!drag) {
            if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < DRAG_THRESHOLD) return;
            drag = this.startDrag(id, dock, title);
          }
          this.dragMove(drag, ev.clientX, ev.clientY);
        };
        const up = () => {
          window.removeEventListener('pointermove', move);
          window.removeEventListener('pointerup', up);
          if (drag) this.endDrag(drag);
        };
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', up);
      });
    },

    startDrag(id, dock, title) {
      dock.classList.add('dragging');
      document.body.classList.add('dock-dragging');
      const ghost = el('div', { class: 'dock-ghost', text: title.firstChild ? title.firstChild.textContent.trim() : id });
      const indicator = el('div', { class: 'drop-indicator', hidden: true });
      document.body.append(ghost, indicator);
      return { id, dock, ghost, indicator, target: null };
    },

    /** Zone et position d'insertion sous le pointeur. */
    findTarget(id, x, y) {
      const main = MS.$('#main').getBoundingClientRect();
      const zones = [];
      for (const zone of ['bottom', 'side']) {
        const z = this.zones[zone];
        let r = z.hidden ? null : z.getBoundingClientRect();
        // Zone vide : bande de dépôt au bord de l'aperçu pour pouvoir la recréer.
        if (!r || !r.width || !r.height) {
          r = zone === 'bottom' ? { left: main.left, right: main.right, top: main.bottom - 70, bottom: main.bottom + 10, width: main.width, height: 80 } : { left: main.right - 90, right: main.right, top: main.top, bottom: main.bottom, width: 90, height: main.height };
        }
        zones.push({ zone, r });
      }
      // La zone de droite est prioritaire là où les deux se touchent.
      const hit = zones.reverse().find(({ r }) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom);
      if (!hit) return null;
      const horizontal = hit.zone === 'bottom';
      const docks = this.current[hit.zone].filter((d) => d !== id && MS.store.settings.ui.docks[d]);
      const rects = docks.map((d) => this.docks[d].getBoundingClientRect());
      let index = rects.findIndex((r) => (horizontal ? x < r.left + r.width / 2 : y < r.top + r.height / 2));
      if (index < 0) index = docks.length;
      // Position de la ligne d'insertion.
      let line;
      if (!rects.length) line = horizontal ? { x: hit.r.left + 4, y: hit.r.top, w: hit.r.width - 8, h: 4, fill: true, r: hit.r } : { x: hit.r.left, y: hit.r.top + 4, w: 4, h: hit.r.height - 8, fill: true, r: hit.r };
      else if (horizontal) {
        const ref = index < rects.length ? rects[index].left - 3 : rects[rects.length - 1].right + 1;
        line = { x: ref, y: hit.r.top + 4, w: 3, h: hit.r.height - 8 };
      } else {
        const ref = index < rects.length ? rects[index].top - 3 : rects[rects.length - 1].bottom + 1;
        line = { x: hit.r.left + 4, y: ref, w: hit.r.width - 8, h: 3 };
      }
      return { zone: hit.zone, before: docks[index] || null, line };
    },

    dragMove(drag, x, y) {
      drag.ghost.style.transform = `translate(${x + 12}px, ${y + 10}px)`;
      const t = this.findTarget(drag.id, x, y);
      drag.target = t;
      const ind = drag.indicator;
      ind.hidden = !t;
      if (!t) return;
      const { line } = t;
      if (line.fill) {
        Object.assign(ind.style, { left: `${line.r.left}px`, top: `${line.r.top}px`, width: `${line.r.width}px`, height: `${line.r.height}px` });
        ind.classList.add('area');
      } else {
        Object.assign(ind.style, { left: `${line.x}px`, top: `${line.y}px`, width: `${line.w}px`, height: `${line.h}px` });
        ind.classList.remove('area');
      }
    },

    endDrag(drag) {
      drag.dock.classList.remove('dragging');
      document.body.classList.remove('dock-dragging');
      drag.ghost.remove();
      drag.indicator.remove();
      const t = drag.target;
      if (!t) return;
      const l = this.current;
      l.bottom = l.bottom.filter((d) => d !== drag.id);
      l.side = l.side.filter((d) => d !== drag.id);
      const list = l[t.zone];
      const at = t.before ? list.indexOf(t.before) : -1;
      if (at < 0) list.push(drag.id);
      else list.splice(at, 0, drag.id);
      MS.store.save();
      this.apply();
    },

    /* Dispositions enregistrées ------------------------------------------- */
    menuItems() {
      const ui = MS.store.settings.ui;
      const names = Object.keys(ui.layouts || {});
      return [
        { separator: true },
        ...names.map((n) => ({ label: `Disposition « ${n} »`, action: () => this.load(n) })),
        { label: 'Enregistrer la disposition…', action: () => this.saveAs() },
        { label: 'Supprimer une disposition…', disabled: !names.length, action: () => this.remove() },
        { label: "Disposition d'origine", action: () => this.reset() },
      ];
    },

    async saveAs() {
      const name = await MS.prompt('Enregistrer la disposition', 'Nom (ex. Live jeu, Discussion)', '');
      if (!name) return;
      const ui = MS.store.settings.ui;
      ui.layouts[name] = { layout: JSON.parse(JSON.stringify(this.current)), docks: { ...ui.docks } };
      MS.store.save();
      MS.toast(`Disposition « ${name} » enregistrée.`, 'success', 3000);
    },

    load(name) {
      const ui = MS.store.settings.ui;
      const saved = ui.layouts[name];
      if (!saved) return;
      ui.layout = normalize(JSON.parse(JSON.stringify(saved.layout)));
      Object.assign(ui.docks, saved.docks || {});
      MS.store.save();
      this.apply();
    },

    async remove() {
      const ui = MS.store.settings.ui;
      const names = Object.keys(ui.layouts);
      const select = el('select', { class: 'input' }, names.map((n) => el('option', { value: n, text: n })));
      const ok = await MS.modal({ title: 'Supprimer une disposition', content: el('label', { class: 'field' }, el('span', { text: 'Disposition' }), select), width: 380 });
      if (!ok) return;
      delete ui.layouts[select.value];
      MS.store.save();
    },

    reset() {
      const ui = MS.store.settings.ui;
      ui.layout = defaultLayout();
      for (const id of DOCK_IDS) ui.docks[id] = true;
      MS.store.save();
      this.apply();
    },
  };

  MS.layout = layout;
})();
