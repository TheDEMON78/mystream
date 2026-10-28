'use strict';

(function () {
  const MS = window.MS;

  MS.filterDefaults = () => ({
    opacity: 100,
    brightness: 100,
    contrast: 100,
    saturation: 100,
    hue: 0,
    cropLeft: 0,
    cropTop: 0,
    cropRight: 0,
    cropBottom: 0,
    flipH: false,
    flipV: false,
    chroma: false,
    chromaColor: '#00ff00',
    similarity: 400,
    smoothness: 80,
  });

  function filterCss(f) {
    const parts = [];
    if (f.brightness !== 100) parts.push(`brightness(${f.brightness}%)`);
    if (f.contrast !== 100) parts.push(`contrast(${f.contrast}%)`);
    if (f.saturation !== 100) parts.push(`saturate(${f.saturation}%)`);
    if (f.hue) parts.push(`hue-rotate(${f.hue}deg)`);
    return parts.join(' ');
  }

  /* Incrustation (fond vert) sur le GPU ---------------------------------- */
  const CHROMA_FS = `
    precision mediump float;
    varying vec2 uv;
    uniform sampler2D tex;
    uniform vec2 keyCbCr;
    uniform float similarity;
    uniform float smoothness;
    vec2 cbcr(vec3 c) {
      return vec2(-0.1687 * c.r - 0.3313 * c.g + 0.5 * c.b, 0.5 * c.r - 0.4187 * c.g - 0.0813 * c.b);
    }
    void main() {
      vec4 c = texture2D(tex, uv);
      float d = distance(cbcr(c.rgb), keyCbCr);
      float a = c.a * smoothstep(similarity, similarity + smoothness, d);
      // Atténue le reflet de la couleur clé sur les bords du sujet.
      float spill = 1.0 - smoothstep(similarity, similarity + smoothness * 4.0, d);
      vec3 rgb = mix(c.rgb, vec3(dot(c.rgb, vec3(0.299, 0.587, 0.114))), spill * 0.6);
      gl_FragColor = vec4(rgb * a, a);
    }`;
  const CHROMA_VS = `
    attribute vec2 pos;
    varying vec2 uv;
    void main() { uv = vec2((pos.x + 1.0) / 2.0, (1.0 - pos.y) / 2.0); gl_Position = vec4(pos, 0.0, 1.0); }`;

  class ChromaKey {
    constructor() {
      this.canvas = document.createElement('canvas');
      const gl = (this.gl = this.canvas.getContext('webgl', { premultipliedAlpha: true, preserveDrawingBuffer: true }));
      if (!gl) {
        MS.toast("Incrustation indisponible : l'accélération graphique (WebGL) est désactivée sur ce PC.", 'error', 8000);
        return;
      }
      const sh = (type, src) => {
        const s = gl.createShader(type);
        gl.shaderSource(s, src);
        gl.compileShader(s);
        return s;
      };
      const prog = gl.createProgram();
      gl.attachShader(prog, sh(gl.VERTEX_SHADER, CHROMA_VS));
      gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, CHROMA_FS));
      gl.linkProgram(prog);
      gl.useProgram(prog);
      gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
      const loc = gl.getAttribLocation(prog, 'pos');
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
      gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      this.u = {
        key: gl.getUniformLocation(prog, 'keyCbCr'),
        sim: gl.getUniformLocation(prog, 'similarity'),
        smooth: gl.getUniformLocation(prog, 'smoothness'),
      };
    }

    process(source, w, h, f) {
      const gl = this.gl;
      if (!gl) return source;
      if (this.canvas.width !== w || this.canvas.height !== h) {
        this.canvas.width = w;
        this.canvas.height = h;
        gl.viewport(0, 0, w, h);
      }
      const n = parseInt(f.chromaColor.slice(1), 16);
      const r = ((n >> 16) & 255) / 255;
      const g = ((n >> 8) & 255) / 255;
      const b = (n & 255) / 255;
      gl.uniform2f(this.u.key, -0.1687 * r - 0.3313 * g + 0.5 * b, 0.5 * r - 0.4187 * g - 0.0813 * b);
      gl.uniform1f(this.u.sim, f.similarity / 1000);
      gl.uniform1f(this.u.smooth, Math.max(0.001, f.smoothness / 1000));
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      return this.canvas;
    }
  }

  /**
   * Planification prioritaire : avec scheduler.postTask « user-blocking », la
   * prochaine image passe avant les autres tâches de l'interface (chat, IPC…).
   */
  function schedule(fn, delay) {
    if (globalThis.scheduler && globalThis.TaskController) {
      const ctl = new TaskController({ priority: 'user-blocking' });
      scheduler.postTask(fn, { delay, signal: ctl.signal }).catch(() => {});
      return () => ctl.abort();
    }
    const t = setTimeout(fn, delay);
    return () => clearTimeout(t);
  }

  /* Compositeur ------------------------------------------------------------ */
  const compositor = {
    canvas: null,
    ctx: null,
    studioCanvas: null,
    studioCtx: null,
    timer: null,
    transition: null,
    chromas: new Map(),
    stats: { fps: 0, renderMs: 0, lagged: 0, total: 0 },

    init(canvas, studioCanvas) {
      this.canvas = canvas;
      this.ctx = canvas.getContext('2d', { alpha: false });
      this.studioCanvas = studioCanvas;
      this.studioCtx = studioCanvas.getContext('2d', { alpha: false });
      MS.on('runtime:disposed', (id) => this.chromas.delete(id));
      this.applyVideoSettings();
    },

    applyVideoSettings() {
      const v = MS.store.settings.video;
      this.canvas.width = v.width;
      this.canvas.height = v.height;
      this.start(v.fps);
      MS.emit('video:resized');
    },

    /**
     * Cadence exacte : chaque image est planifiée à t0 + n × intervalle.
     * setInterval arrondit l'intervalle à la milliseconde (16 ms au lieu de
     * 16,67 ms), ce qui produisait 62,5 images/s au lieu de 60.
     */
    start(fps) {
      if (this.cancel) this.cancel();
      this.fps = fps;
      this.interval = 1000 / fps;
      this.next = performance.now();
      this.window = { frames: 0, start: this.next };
      const tick = () => {
        const now = performance.now();
        const late = now - this.next;
        if (late >= this.interval) {
          // Images impossibles à rattraper : on les compte et on se recale.
          const missed = Math.floor(late / this.interval);
          this.stats.lagged += missed;
          this.next += missed * this.interval;
        }
        try {
          // Fenêtre réduite et aucune sortie active : personne ne voit l'image, on ne la calcule pas.
          if (!document.hidden || MS.outputs.isAnyActive()) this.frame();
        } catch (e) {
          // Une source défaillante ne doit jamais interrompre la boucle de rendu.
          if (e.message !== this.lastError) console.error('Rendu :', e);
          this.lastError = e.message;
        }
        this.window.frames++;
        if (this.next - this.window.start >= 1000) {
          this.stats.fps = (this.window.frames * 1000) / (this.next - this.window.start + this.interval);
          this.window = { frames: 0, start: this.next + this.interval };
        }
        this.next += this.interval;
        this.cancel = schedule(tick, Math.max(0, this.next - performance.now()));
      };
      tick();
    },

    /** Transition utilisée pour arriver sur une scène : la sienne, sinon celle du dock. */
    transitionFor(id) {
      const scene = MS.store.scene(id);
      return (scene && scene.transition) || MS.store.settings.transition;
    },

    switchScene(id) {
      const from = MS.store.data.currentSceneId;
      if (from === id) return;
      const t = this.transitionFor(id);
      const now = performance.now();
      this.transition = null;
      if (from && t.type === 'stinger') {
        const st = this.stinger(t.stingerFile);
        if (st && st.gif && st.gif.duration) {
          st.gif.restart();
          this.transition = { type: 'stinger', from, start: now, gif: st.gif, point: Number(t.stingerPoint) || 0, duration: st.gif.duration };
        } else if (st && st.video && st.video.readyState >= 2 && st.video.duration) {
          const v = st.video;
          v.muted = !t.stingerAudio;
          v.currentTime = 0;
          v.play().catch(() => {});
          this.transition = { type: 'stinger', from, start: now, video: v, point: Number(t.stingerPoint) || 0, duration: v.duration * 1000 };
        }
      } else if (from && t.type !== 'cut' && Number(t.duration) > 0) {
        this.transition = { type: t.type, direction: t.direction, from, start: now, duration: Number(t.duration) };
      }
      MS.store.setCurrentScene(id);
    },

    /**
     * Vidéos des stingers, chargées à l'avance pour démarrer sans délai.
     * Leur son passe dans le mixage du live (pas dans le retour casque).
     */
    stinger(file) {
      if (!file) return null;
      this.stingers = this.stingers || new Map();
      let st = this.stingers.get(file);
      if (st) return st;
      st = {};
      this.stingers.set(file, st);
      // Stinger en GIF (fréquent dans les packs pour OBS) : pas de son.
      if (MS.isGif(file)) {
        new MS.GifPlayer(MS.fileUrl(file))
          .load()
          .then((g) => (st.gif = g))
          .catch((e) => MS.toast(`Stinger illisible : ${file} (${e.message})`, 'error', 8000));
        return st;
      }
      const v = document.createElement('video');
      v.preload = 'auto';
      v.playsInline = true;
      v.src = MS.fileUrl(file);
      v.addEventListener('error', () => MS.toast(`Vidéo du stinger illisible : ${file}`, 'error', 8000), { once: true });
      try {
        const node = MS.audio.ctx.createMediaElementSource(v);
        node.connect(MS.audio.dest);
      } catch (e) {
        console.warn('Son du stinger indisponible', e);
      }
      v.load();
      st.video = v;
      return st;
    },

    preloadStingers() {
      const files = new Set();
      const add = (t) => t && t.type === 'stinger' && t.stingerFile && files.add(t.stingerFile);
      add(MS.store.settings.transition);
      MS.store.data.scenes.forEach((s) => add(s.transition));
      files.forEach((f) => this.stinger(f));
    },

    /** Dessine une transition en cours ; renvoie false quand elle est terminée. */
    drawTransition(ctx, tr, current, now) {
      const { width: W, height: H } = MS.store.settings.video;
      const from = MS.store.scene(tr.from);
      const elapsed = now - tr.start;
      if (tr.type === 'stinger') {
        if (elapsed > tr.duration + 250 || (tr.video && tr.video.ended)) return false;
        this.drawScene(ctx, elapsed < tr.point ? from : current, 1);
        const img = tr.gif ? tr.gif.current(false) : tr.video.readyState >= 2 ? tr.video : null;
        if (img) ctx.drawImage(img, 0, 0, W, H);
        return true;
      }
      const p = elapsed / tr.duration;
      if (p >= 1) return false;
      const e = p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2; // ease-in-out
      if (tr.type === 'slide') {
        const [dx, dy] = { left: [-1, 0], right: [1, 0], up: [0, -1], down: [0, 1] }[tr.direction] || [-1, 0];
        ctx.save();
        ctx.translate(dx * W * e, dy * H * e);
        this.drawScene(ctx, from, 1);
        ctx.translate(-dx * W, -dy * H);
        this.drawScene(ctx, current, 1);
        ctx.restore();
      } else if (tr.type === 'zoom') {
        const zoom = (scene, scale, alpha) => {
          ctx.save();
          ctx.translate(W / 2, H / 2);
          ctx.scale(scale, scale);
          ctx.translate(-W / 2, -H / 2);
          this.drawScene(ctx, scene, alpha);
          ctx.restore();
        };
        zoom(from, 1 + 0.25 * e, 1 - e);
        zoom(current, 0.8 + 0.2 * e, e);
      } else {
        this.drawScene(ctx, from, 1);
        this.drawScene(ctx, current, p);
      }
      return true;
    },

    /** Dimensions de la source après recadrage. */
    sourceSize(src, rt) {
      const f = src && src.filters;
      if (!f) return { w: rt.width, h: rt.height };
      return {
        w: Math.max(1, rt.width - f.cropLeft - f.cropRight),
        h: Math.max(1, rt.height - f.cropTop - f.cropBottom),
      };
    },

    /** Taille affichée d'un élément, ou null tant que la source n'a pas de dimensions. */
    itemRect(item) {
      const rt = MS.runtimes.get(item.sourceId);
      if (!rt || !rt.width || !rt.height) return null;
      const { w, h } = this.sourceSize(MS.store.source(item.sourceId), rt);
      if (item.scaleX == null) this.initItem(item, w, h);
      return { x: item.x, y: item.y, w: w * item.scaleX, h: h * item.scaleY, natW: w, natH: h };
    },

    /** Première apparition : comme OBS, on place l'élément en haut à gauche et on le réduit s'il dépasse. */
    initItem(item, w, h) {
      if (item.obsPlace) {
        Object.assign(item, MS.obsPlacement(item.obsPlace, w, h));
        delete item.obsPlace;
      } else {
        const { width, height } = MS.store.settings.video;
        const s = Math.min(1, width / w, height / h);
        item.scaleX = s;
        item.scaleY = s;
      }
      MS.store.save();
      MS.emit('items:changed');
    },

    drawScene(ctx, scene, alpha) {
      if (!scene) return;
      const { width: W, height: H } = MS.store.settings.video;
      for (let i = scene.items.length - 1; i >= 0; i--) {
        const item = scene.items[i];
        if (!item.visible) continue;
        const rt = MS.runtimes.get(item.sourceId);
        let d = rt && rt.drawable();
        if (!d) continue;
        const r = this.itemRect(item);
        if (!r || r.w <= 0 || r.h <= 0 || r.x >= W || r.y >= H || r.x + r.w <= 0 || r.y + r.h <= 0) continue;
        const f = MS.store.source(item.sourceId).filters;
        if (!f) {
          ctx.globalAlpha = alpha;
          ctx.drawImage(d, r.x, r.y, r.w, r.h);
          continue;
        }
        if (f.chroma) {
          let ck = this.chromas.get(item.sourceId);
          if (!ck) this.chromas.set(item.sourceId, (ck = new ChromaKey()));
          d = ck.process(d, rt.width, rt.height, f);
        }
        ctx.save();
        ctx.globalAlpha = (alpha * f.opacity) / 100;
        const css = filterCss(f);
        if (css) ctx.filter = css;
        let { x, y } = r;
        if (f.flipH || f.flipV) {
          ctx.translate(x + (f.flipH ? r.w : 0), y + (f.flipV ? r.h : 0));
          ctx.scale(f.flipH ? -1 : 1, f.flipV ? -1 : 1);
          x = 0;
          y = 0;
        }
        ctx.drawImage(d, f.cropLeft, f.cropTop, r.natW, r.natH, x, y, r.w, r.h);
        ctx.restore();
      }
      ctx.globalAlpha = 1;
    },

    frame() {
      const t0 = performance.now();
      this.stats.total++;
      const ctx = this.ctx;
      ctx.globalAlpha = 1;
      ctx.fillStyle = '#000';
      ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);

      const current = MS.store.currentScene();
      if (this.transition && !this.drawTransition(ctx, this.transition, current, t0)) {
        if (this.transition.gif) this.transition.gif.prepare();
        this.transition = null;
      }
      if (!this.transition) this.drawScene(ctx, current, 1);

      // Mode studio : l'aperçu est rendu directement à la taille affichée.
      if (MS.store.settings.ui.studio && this.studioCanvas.width > 1) {
        const sc = this.studioCtx;
        sc.setTransform(1, 0, 0, 1, 0, 0);
        sc.globalAlpha = 1;
        sc.fillStyle = '#000';
        sc.fillRect(0, 0, this.studioCanvas.width, this.studioCanvas.height);
        const k = this.studioCanvas.width / this.canvas.width;
        sc.setTransform(k, 0, 0, k, 0, 0);
        this.drawScene(sc, MS.store.editScene(), 1);
      }

      this.stats.renderMs = this.stats.renderMs * 0.9 + (performance.now() - t0) * 0.1;
    },
  };

  MS.compositor = compositor;
})();
