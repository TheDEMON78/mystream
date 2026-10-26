'use strict';

(function () {
  const MS = window.MS;

  /* Moteurs d'exécution des sources ----------------------------------- */

  class BaseRuntime {
    constructor(src) {
      this.src = src;
      this.error = null;
      this.ready = false;
      this.disposed = false;
    }
    get width() {
      return 0;
    }
    get height() {
      return 0;
    }
    drawable() {
      return null;
    }
    hasAudio() {
      return false;
    }
    audioNode() {
      return null;
    }
    async start() {}
    markReady() {
      if (this.disposed) return;
      this.ready = true;
      this.error = null;
      MS.emit('runtime:ready', this);
    }
    fail(err) {
      if (this.disposed) return;
      this.error = (err && err.message) || String(err);
      console.warn(`Source « ${this.src.name} » :`, err);
      MS.emit('runtime:error', this);
    }
    dispose() {
      this.disposed = true;
    }
  }

  class StreamRuntime extends BaseRuntime {
    async start() {
      try {
        const stream = await this.getStream();
        if (this.disposed) {
          stream.getTracks().forEach((t) => t.stop());
          return;
        }
        this.stream = stream;
        const track = stream.getVideoTracks()[0] || stream.getAudioTracks()[0];
        if (track) track.addEventListener('ended', () => this.fail(new Error('Capture interrompue')));
        if (stream.getVideoTracks().length) {
          const v = document.createElement('video');
          v.muted = true;
          v.playsInline = true;
          v.srcObject = stream;
          await v.play().catch(() => {});
          this.video = v;
        }
        this.markReady();
      } catch (e) {
        this.fail(e);
      }
    }
    get width() {
      return this.video ? this.video.videoWidth : 0;
    }
    get height() {
      return this.video ? this.video.videoHeight : 0;
    }
    drawable() {
      return this.video && this.video.readyState >= 2 ? this.video : null;
    }
    hasAudio() {
      return Boolean(this.stream && this.stream.getAudioTracks().length);
    }
    audioNode(ctx) {
      if (!this.node) this.node = ctx.createMediaStreamSource(this.stream);
      return this.node;
    }
    dispose() {
      super.dispose();
      if (this.stream) this.stream.getTracks().forEach((t) => t.stop());
      if (this.video) this.video.srcObject = null;
    }
  }

  class VideoCaptureRuntime extends StreamRuntime {
    getStream() {
      const s = this.src.settings;
      const video = {};
      if (s.deviceId) video.deviceId = { exact: s.deviceId };
      if (s.resolution && s.resolution !== 'default') {
        const [w, h] = s.resolution.split('x').map(Number);
        video.width = { ideal: w };
        video.height = { ideal: h };
      }
      if (s.fps && s.fps !== 'default') video.frameRate = { ideal: Number(s.fps) };
      return navigator.mediaDevices.getUserMedia({ video, audio: false });
    }
  }

  class DisplayCaptureRuntime extends StreamRuntime {
    async getStream() {
      const s = this.src.settings;
      const list = await window.api.getCaptureSources(false);
      let target = list.find((x) => x.id === s.captureId);
      // Les identifiants de fenêtres changent d'une session à l'autre : on retrouve la fenêtre par son nom.
      if (!target && s.captureName) target = list.find((x) => x.name === s.captureName && x.kind === s.captureKind);
      if (!target && s.captureKind === 'screen') target = list.find((x) => x.kind === 'screen');
      if (!target) throw new Error('Écran ou fenêtre introuvable');
      s.captureId = target.id;
      const { width, height, fps } = MS.store.settings.video;
      return navigator.mediaDevices.getUserMedia({
        audio: false,
        video: {
          mandatory: {
            chromeMediaSource: 'desktop',
            chromeMediaSourceId: target.id,
            // L'écran est réduit dès la capture à la taille du canevas : bien
            // moins de données à copier et à redimensionner à chaque image.
            maxWidth: width,
            maxHeight: height,
            maxFrameRate: fps,
          },
        },
      });
    }
  }

  class AudioInputRuntime extends StreamRuntime {
    getStream() {
      const id = this.src.settings.deviceId;
      return navigator.mediaDevices.getUserMedia({
        video: false,
        audio: {
          deviceId: id && id !== 'default' ? { exact: id } : undefined,
          echoCancellation: false,
          noiseSuppression: Boolean(this.src.settings.noiseSuppression),
          autoGainControl: false,
        },
      });
    }
  }

  class DesktopAudioRuntime extends StreamRuntime {
    async getStream() {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { mandatory: { chromeMediaSource: 'desktop' } },
        video: { mandatory: { chromeMediaSource: 'desktop', maxWidth: 1, maxHeight: 1 } },
      });
      for (const t of stream.getVideoTracks()) {
        t.stop();
        stream.removeTrack(t);
      }
      if (!stream.getAudioTracks().length) throw new Error('Capture audio du bureau non prise en charge sur ce système');
      return stream;
    }
  }

  class ImageRuntime extends BaseRuntime {
    async start() {
      const img = new Image();
      img.onload = () => this.markReady();
      img.onerror = () => this.fail(new Error('Image illisible'));
      this.img = img;
      if (this.src.settings.file) img.src = MS.fileUrl(this.src.settings.file);
    }
    get width() {
      return this.img ? this.img.naturalWidth : 0;
    }
    get height() {
      return this.img ? this.img.naturalHeight : 0;
    }
    drawable() {
      return this.ready ? this.img : null;
    }
  }

  class MediaRuntime extends BaseRuntime {
    async start() {
      const s = this.src.settings;
      if (!s.file) return;
      const v = document.createElement('video');
      v.loop = Boolean(s.loop);
      v.playsInline = true;
      v.preload = 'auto';
      v.src = MS.fileUrl(s.file);
      v.addEventListener('loadeddata', () => this.markReady(), { once: true });
      v.addEventListener('error', () => this.fail(new Error('Fichier média illisible')));
      this.video = v;
      v.play().catch(() => {});
    }
    get width() {
      return this.video ? this.video.videoWidth : 0;
    }
    get height() {
      return this.video ? this.video.videoHeight : 0;
    }
    drawable() {
      return this.video && this.video.readyState >= 2 && this.video.videoWidth ? this.video : null;
    }
    hasAudio() {
      return Boolean(this.video);
    }
    audioNode(ctx) {
      if (!this.node) this.node = ctx.createMediaElementSource(this.video);
      return this.node;
    }
    restart() {
      if (!this.video) return;
      this.video.currentTime = 0;
      this.video.play().catch(() => {});
    }
    dispose() {
      super.dispose();
      if (this.video) {
        this.video.pause();
        this.video.removeAttribute('src');
        this.video.load();
      }
    }
  }

  class CanvasRuntime extends BaseRuntime {
    async start() {
      this.canvas = document.createElement('canvas');
      this.update();
      this.markReady();
    }
    get width() {
      return this.canvas ? this.canvas.width : 0;
    }
    get height() {
      return this.canvas ? this.canvas.height : 0;
    }
    drawable() {
      return this.canvas;
    }
  }

  class TextRuntime extends CanvasRuntime {
    update() {
      const s = this.src.settings;
      const c = this.canvas;
      let ctx = c.getContext('2d');
      const size = Math.max(4, Number(s.size) || 64);
      const font = `${s.italic ? 'italic ' : ''}${s.bold ? 'bold ' : ''}${size}px "${s.font}", sans-serif`;
      ctx.font = font;
      const lines = String(s.text || ' ').split('\n');
      const lineHeight = Math.ceil(size * 1.25);
      const outline = s.outline ? Math.max(1, Number(s.outlineSize) || 2) : 0;
      const pad = outline + (s.bgOpacity > 0 ? Math.round(size * 0.25) : 0);
      const widths = lines.map((l) => ctx.measureText(l).width);
      const textW = Math.ceil(Math.max(1, ...widths));
      c.width = textW + pad * 2;
      c.height = lineHeight * lines.length + pad * 2;
      ctx = c.getContext('2d');
      if (s.bgOpacity > 0) {
        ctx.fillStyle = MS.hexToRgba(s.bgColor, s.bgOpacity / 100);
        ctx.fillRect(0, 0, c.width, c.height);
      }
      ctx.font = font;
      ctx.textBaseline = 'middle';
      ctx.lineJoin = 'round';
      lines.forEach((line, i) => {
        let x = pad;
        if (s.align === 'center') x = pad + (textW - widths[i]) / 2;
        else if (s.align === 'right') x = pad + textW - widths[i];
        const y = pad + i * lineHeight + lineHeight / 2;
        if (outline) {
          ctx.lineWidth = outline * 2;
          ctx.strokeStyle = s.outlineColor;
          ctx.strokeText(line, x, y);
        }
        ctx.fillStyle = s.color;
        ctx.fillText(line, x, y);
      });
    }
  }

  class ColorRuntime extends CanvasRuntime {
    update() {
      const s = this.src.settings;
      this.canvas.width = Math.max(1, Number(s.width) || 1);
      this.canvas.height = Math.max(1, Number(s.height) || 1);
      const ctx = this.canvas.getContext('2d');
      ctx.fillStyle = s.color;
      ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
    }
  }

  /** Chat incrusté : dessiné nativement, mis à jour uniquement à l'arrivée d'un message. */
  class ChatRuntime extends CanvasRuntime {
    async start() {
      await super.start();
      // Plusieurs messages arrivés ensemble = un seul redessin.
      this.onMessage = () => {
        if (this.pending) return;
        this.pending = setTimeout(() => {
          this.pending = null;
          this.update();
        }, 50);
      };
      for (const evt of ['chat:message', 'chat:cleared', 'chat:image']) MS.on(evt, this.onMessage);
      this.timer = setInterval(() => this.src.settings.lifetime > 0 && this.update(), 250);
    }

    update() {
      const s = this.src.settings;
      const c = this.canvas;
      const W = Math.max(50, Number(s.width) || 400);
      const H = Math.max(50, Number(s.height) || 600);
      if (c.width !== W || c.height !== H) {
        c.width = W;
        c.height = H;
      }
      const ctx = c.getContext('2d');
      ctx.clearRect(0, 0, W, H);
      if (s.bgOpacity > 0) {
        ctx.fillStyle = MS.hexToRgba(s.bgColor, s.bgOpacity / 100);
        ctx.fillRect(0, 0, W, H);
      }
      const size = Math.max(8, Number(s.fontSize) || 22);
      const lineH = Math.round(size * 1.4);
      const pad = Math.round(size * 0.5);
      const font = (bold) => `${bold ? '700' : '400'} ${size}px "${s.font}", sans-serif`;
      const now = Date.now();
      const life = (Number(s.lifetime) || 0) * 1000;

      let y = H - pad;
      const msgs = MS.chat ? MS.chat.messages : [];
      for (let i = msgs.length - 1; i >= 0 && y > 0; i--) {
        const m = msgs[i];
        if (m.divider || (m.system && !s.showSystem)) continue;
        let alpha = 1;
        if (life > 0) {
          const age = now - m.at;
          if (!m.at || age > life) continue;
          alpha = Math.min(1, (life - age) / 1000);
        }
        const rows = this.layout(ctx, m, W - pad * 2, size, font);
        if (y - rows.length * lineH < 0) break; // pas de message coupé en haut
        y -= rows.length * lineH;
        ctx.globalAlpha = alpha;
        rows.forEach((row, r) => {
          for (const seg of row) this.drawSegment(ctx, seg, pad, y + r * lineH + lineH / 2, size, font);
        });
        y -= Math.round(size * 0.3);
      }
      ctx.globalAlpha = 1;
    }

    /** Découpe un message en lignes de segments (logo, badges, pseudo, mots, emotes). */
    layout(ctx, m, maxW, size, font) {
      const s = this.src.settings;
      const tokens = [];
      if (m.system) {
        tokens.push({ kind: 'text', text: `[${m.platform}]`, color: '#9aa0b4', bold: true });
      } else {
        if (s.showIcons && MS.PLATFORM_ICONS[m.platform]) tokens.push({ kind: 'icon', platform: m.platform, w: size });
        if (s.showBadges) {
          for (const b of m.badges || []) {
            if (b.imageUrl) tokens.push({ kind: 'img', url: b.imageUrl, w: size });
            else if (MS.BADGE_ICONS[b.type]) tokens.push({ kind: 'badgeicon', type: b.type, w: size });
            else {
              const label = MS.BADGE_LABELS[b.type] || b.title || b.type;
              ctx.font = `700 ${Math.round(size * 0.6)}px sans-serif`;
              tokens.push({ kind: 'badge', text: label, w: ctx.measureText(label).width + size * 0.5 });
            }
          }
        }
        tokens.push({ kind: 'text', text: m.username, color: m.color || s.nameColor, bold: true });
        tokens.push({ kind: 'text', text: ':', color: s.textColor, bold: false });
      }
      for (const p of m.parts || [{ type: 'text', text: m.message }]) {
        if (p.type === 'emote') {
          const img = MS.chatImage(p.url);
          const h = size * 1.3;
          tokens.push({ kind: 'img', url: p.url, w: img ? (img.naturalWidth / img.naturalHeight) * h : h, h });
        } else {
          for (const word of String(p.text || '').split(/\s+/).filter(Boolean)) tokens.push({ kind: 'text', text: word, color: s.textColor, bold: false });
        }
      }

      const gap = size * 0.3;
      const rows = [[]];
      let x = 0;
      for (const t of tokens) {
        if (t.kind === 'text') {
          ctx.font = font(t.bold);
          t.w = ctx.measureText(t.text).width;
        }
        const space = x > 0 && !(t.text === ':') ? gap : 0;
        if (x > 0 && x + space + t.w > maxW) {
          rows.push([]);
          x = 0;
        } else {
          x += space;
        }
        t.x = x;
        rows[rows.length - 1].push(t);
        x += t.w;
      }
      return rows;
    }

    drawSegment(ctx, t, ox, cy, size, font) {
      const x = ox + t.x;
      if (t.kind === 'icon') {
        ctx.save();
        ctx.translate(x, cy - size / 2);
        ctx.scale(size / 24, size / 24);
        ctx.fillStyle = MS.PLATFORM_COLORS[t.platform];
        ctx.fill(new Path2D(MS.PLATFORM_ICONS[t.platform]));
        ctx.restore();
      } else if (t.kind === 'img') {
        const img = MS.chatImage(t.url);
        const h = t.h || size;
        if (img) ctx.drawImage(img, x, cy - h / 2, t.w, h);
      } else if (t.kind === 'badgeicon') {
        const ic = MS.BADGE_ICONS[t.type];
        ctx.save();
        ctx.translate(x, cy - size / 2);
        ctx.scale(size / 24, size / 24);
        ctx.fillStyle = ic.bg;
        ctx.beginPath();
        ctx.roundRect(0, 0, 24, 24, 5);
        ctx.fill();
        ctx.strokeStyle = '#fff';
        ctx.lineWidth = 2;
        ctx.lineJoin = 'round';
        ctx.lineCap = 'round';
        ctx.stroke(new Path2D(ic.d));
        ctx.restore();
      } else if (t.kind === 'badge') {
        const h = size * 0.85;
        ctx.fillStyle = 'rgba(76, 125, 255, 0.85)';
        ctx.beginPath();
        ctx.roundRect(x, cy - h / 2, t.w, h, h / 4);
        ctx.fill();
        ctx.fillStyle = '#fff';
        ctx.font = `700 ${Math.round(size * 0.6)}px sans-serif`;
        ctx.textBaseline = 'middle';
        ctx.fillText(t.text, x + size * 0.25, cy);
      } else {
        ctx.font = font(t.bold);
        ctx.textBaseline = 'middle';
        // Contour plutôt qu'ombre floue : aussi lisible, bien moins coûteux.
        if (this.src.settings.shadow) {
          ctx.lineWidth = Math.max(2, size * 0.16);
          ctx.lineJoin = 'round';
          ctx.strokeStyle = 'rgba(0, 0, 0, 0.85)';
          ctx.strokeText(t.text, x, cy);
        }
        ctx.fillStyle = t.color;
        ctx.fillText(t.text, x, cy);
      }
    }

    dispose() {
      super.dispose();
      clearInterval(this.timer);
      clearTimeout(this.pending);
      for (const evt of ['chat:message', 'chat:cleared', 'chat:image']) MS.off(evt, this.onMessage);
    }
  }

  /** Page web rendue hors écran par le processus principal. */
  class BrowserRuntime extends BaseRuntime {
    async start() {
      this.canvas = document.createElement('canvas');
      this.ctx = this.canvas.getContext('2d');
      this.signature = this.sig();
      window.api.browserCreate(this.src.id, { ...this.src.settings });
    }
    sig() {
      const s = this.src.settings;
      return JSON.stringify([s.url, s.width, s.height, s.fps, s.css, s.audio]);
    }
    update() {
      if (this.sig() === this.signature) return;
      this.signature = this.sig();
      window.api.browserCreate(this.src.id, { ...this.src.settings });
    }
    /** Reçoit uniquement la zone modifiée, déjà convertie en RGBA par le processus principal. */
    onFrame({ width, height, x, y, w, h, data }) {
      if (this.disposed) return;
      if (this.canvas.width !== width || this.canvas.height !== height) {
        this.canvas.width = width;
        this.canvas.height = height;
      }
      this.ctx.putImageData(new ImageData(new Uint8ClampedArray(data.buffer, data.byteOffset, w * h * 4), w, h), x, y);
      if (!this.ready) this.markReady();
    }
    get width() {
      return this.ready ? this.canvas.width : 0;
    }
    get height() {
      return this.ready ? this.canvas.height : 0;
    }
    drawable() {
      return this.ready ? this.canvas : null;
    }
    dispose() {
      super.dispose();
      window.api.browserDestroy(this.src.id);
    }
  }

  window.api.onBrowserFrame((f) => {
    const rt = MS.runtimes.get(f.id);
    try {
      if (rt && rt.onFrame) rt.onFrame(f);
    } finally {
      // Toujours accuser réception : l'image suivante n'est envoyée qu'après.
      window.api.browserAck(f.id);
    }
  });
  window.api.onBrowserError((e) => {
    const rt = MS.runtimes.get(e.id);
    if (rt) rt.fail(new Error(e.error));
  });

  /* Listes de périphériques ------------------------------------------- */

  async function deviceOptions(kind, withDefault) {
    let devices = await navigator.mediaDevices.enumerateDevices();
    if (devices.some((d) => d.kind === kind && !d.label)) {
      // Les libellés ne sont visibles qu'après une première autorisation.
      try {
        const tmp = await navigator.mediaDevices.getUserMedia(kind === 'videoinput' ? { video: true } : { audio: true });
        tmp.getTracks().forEach((t) => t.stop());
        devices = await navigator.mediaDevices.enumerateDevices();
      } catch {
        /* autorisation refusée : on garde les identifiants bruts */
      }
    }
    const opts = devices
      .filter((d) => d.kind === kind && d.deviceId !== 'default' && d.deviceId !== 'communications')
      .map((d, i) => ({ value: d.deviceId, label: d.label || `Périphérique ${i + 1}` }));
    return withDefault ? [{ value: 'default', label: 'Par défaut' }, ...opts] : opts;
  }
  MS.deviceOptions = deviceOptions;

  const FONTS = ['Arial', 'Segoe UI', 'Roboto', 'Helvetica', 'Verdana', 'Tahoma', 'Trebuchet MS', 'Georgia', 'Times New Roman', 'Courier New', 'Impact', 'Comic Sans MS'];

  /* Types de sources --------------------------------------------------- */

  MS.SourceTypes = {
    display_capture: {
      label: "Capture d'écran / fenêtre",
      glyph: '🖥',
      defaults: () => ({ captureId: '', captureName: '', captureKind: 'screen' }),
      create: (src) => new DisplayCaptureRuntime(src),
      fields: () => [{ key: 'capture', type: 'capture', label: 'Écran ou fenêtre' }],
    },
    video_capture: {
      label: 'Périphérique de capture vidéo',
      glyph: '📷',
      defaults: () => ({ deviceId: '', resolution: 'default', fps: 'default' }),
      create: (src) => new VideoCaptureRuntime(src),
      fields: async () => [
        { key: 'deviceId', type: 'select', label: 'Périphérique', options: await deviceOptions('videoinput', false) },
        {
          key: 'resolution',
          type: 'select',
          label: 'Résolution',
          options: [
            { value: 'default', label: 'Par défaut' },
            { value: '640x480', label: '640 × 480' },
            { value: '1280x720', label: '1280 × 720' },
            { value: '1920x1080', label: '1920 × 1080' },
            { value: '2560x1440', label: '2560 × 1440' },
            { value: '3840x2160', label: '3840 × 2160' },
          ],
        },
        {
          key: 'fps',
          type: 'select',
          label: 'Images par seconde',
          options: [
            { value: 'default', label: 'Par défaut' },
            { value: '30', label: '30' },
            { value: '60', label: '60' },
          ],
        },
      ],
    },
    audio_input: {
      label: "Capture d'entrée audio",
      glyph: '🎤',
      audioOnly: true,
      defaults: () => ({ deviceId: 'default', noiseSuppression: false }),
      create: (src) => new AudioInputRuntime(src),
      fields: async () => [
        { key: 'deviceId', type: 'select', label: 'Périphérique', options: await deviceOptions('audioinput', true) },
        { key: 'noiseSuppression', type: 'checkbox', label: 'Suppression du bruit' },
      ],
    },
    desktop_audio: {
      label: 'Capture audio du bureau',
      glyph: '🔊',
      audioOnly: true,
      defaults: () => ({}),
      create: (src) => new DesktopAudioRuntime(src),
      fields: () => [{ type: 'info', text: 'Capture le son de tout le système. Disponible sous Windows.' }],
    },
    image: {
      label: 'Image',
      glyph: '🖼',
      defaults: () => ({ file: '' }),
      create: (src) => new ImageRuntime(src),
      fields: () => [{ key: 'file', type: 'file', label: 'Fichier image', filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'webp', 'svg'] }] }],
    },
    media: {
      label: 'Source média',
      glyph: '🎞',
      defaults: () => ({ file: '', loop: true }),
      create: (src) => new MediaRuntime(src),
      fields: () => [
        { key: 'file', type: 'file', label: 'Fichier vidéo / audio', filters: [{ name: 'Médias', extensions: ['mp4', 'webm', 'mkv', 'mov', 'mp3', 'ogg', 'wav', 'm4a'] }] },
        { key: 'loop', type: 'checkbox', label: 'Lecture en boucle' },
      ],
    },
    text: {
      label: 'Texte',
      glyph: '🅣',
      live: true,
      defaults: () => ({
        text: 'Texte',
        font: 'Arial',
        size: 72,
        bold: true,
        italic: false,
        color: '#ffffff',
        align: 'left',
        outline: true,
        outlineColor: '#000000',
        outlineSize: 3,
        bgColor: '#000000',
        bgOpacity: 0,
      }),
      create: (src) => new TextRuntime(src),
      fields: () => [
        { key: 'text', type: 'textarea', label: 'Texte' },
        { key: 'font', type: 'select', label: 'Police', options: FONTS.map((f) => ({ value: f, label: f })) },
        { key: 'size', type: 'number', label: 'Taille', min: 4, max: 512 },
        { key: 'color', type: 'color', label: 'Couleur' },
        { key: 'bold', type: 'checkbox', label: 'Gras' },
        { key: 'italic', type: 'checkbox', label: 'Italique' },
        {
          key: 'align',
          type: 'select',
          label: 'Alignement',
          options: [
            { value: 'left', label: 'Gauche' },
            { value: 'center', label: 'Centre' },
            { value: 'right', label: 'Droite' },
          ],
        },
        { key: 'outline', type: 'checkbox', label: 'Contour' },
        { key: 'outlineColor', type: 'color', label: 'Couleur du contour' },
        { key: 'outlineSize', type: 'number', label: 'Épaisseur du contour', min: 1, max: 40 },
        { key: 'bgColor', type: 'color', label: 'Couleur de fond' },
        { key: 'bgOpacity', type: 'range', label: 'Opacité du fond (%)', min: 0, max: 100 },
      ],
    },
    chat: {
      label: 'Chat du stream',
      glyph: '💬',
      live: true,
      defaults: () => ({
        width: 450,
        height: 600,
        font: 'Segoe UI',
        fontSize: 24,
        textColor: '#ffffff',
        nameColor: '#4c7dff',
        bgColor: '#000000',
        bgOpacity: 0,
        lifetime: 0,
        showIcons: true,
        showBadges: true,
        showSystem: false,
        shadow: true,
      }),
      create: (src) => new ChatRuntime(src),
      fields: () => [
        { type: 'info', text: 'Affiche les messages du chat (Paramètres > Chat) directement dans la scène.' },
        { key: 'width', type: 'number', label: 'Largeur', min: 100, max: 3840 },
        { key: 'height', type: 'number', label: 'Hauteur', min: 100, max: 2160 },
        { key: 'font', type: 'select', label: 'Police', options: FONTS.map((f) => ({ value: f, label: f })) },
        { key: 'fontSize', type: 'number', label: 'Taille du texte', min: 8, max: 96 },
        { key: 'textColor', type: 'color', label: 'Couleur du texte' },
        { key: 'nameColor', type: 'color', label: 'Couleur des pseudos (si aucune)' },
        { key: 'bgColor', type: 'color', label: 'Couleur de fond' },
        { key: 'bgOpacity', type: 'range', label: 'Opacité du fond (%)', min: 0, max: 100 },
        { key: 'lifetime', type: 'number', label: 'Masquer les messages après (s, 0 = jamais)', min: 0, max: 3600 },
        { key: 'showIcons', type: 'checkbox', label: 'Logo de la plateforme' },
        { key: 'showBadges', type: 'checkbox', label: 'Badges (modo, abonné…)' },
        { key: 'showSystem', type: 'checkbox', label: 'Messages système' },
        { key: 'shadow', type: 'checkbox', label: 'Contour du texte' },
      ],
    },
    browser: {
      label: 'Navigateur',
      glyph: '🌐',
      defaults: () => ({ url: '', width: 800, height: 600, fps: 30, css: MS.appInfo.defaultBrowserCss, audio: false }),
      create: (src) => new BrowserRuntime(src),
      fields: () => [
        { key: 'url', type: 'text', label: 'URL (ou chemin d’un fichier HTML local)', placeholder: 'https://…' },
        { key: 'width', type: 'number', label: 'Largeur', min: 16, max: 4096 },
        { key: 'height', type: 'number', label: 'Hauteur', min: 16, max: 4096 },
        { key: 'fps', type: 'number', label: 'Images par seconde', min: 1, max: 60 },
        { key: 'audio', type: 'checkbox', label: 'Jouer le son de la page (sortie du PC)' },
        { key: 'css', type: 'textarea', label: 'CSS personnalisé' },
        { type: 'info', text: 'Idéal pour les alertes (StreamElements, Streamlabs…) ou le chat de Restream (/chat.html).' },
      ],
      actions: [{ label: 'Actualiser la page', run: (src) => window.api.browserReload(src.id) }],
    },
    color: {
      label: 'Source de couleur',
      glyph: '🎨',
      live: true,
      defaults: () => ({ color: '#2b6cff', width: MS.store.settings.video.width, height: MS.store.settings.video.height }),
      create: (src) => new ColorRuntime(src),
      fields: () => [
        { key: 'color', type: 'color', label: 'Couleur' },
        { key: 'width', type: 'number', label: 'Largeur', min: 1, max: 8192 },
        { key: 'height', type: 'number', label: 'Hauteur', min: 1, max: 8192 },
      ],
    },
  };

  /* Gestionnaire des moteurs ------------------------------------------- */

  MS.runtimes = {
    map: new Map(),

    get(id) {
      return this.map.get(id) || null;
    },

    ensure(src) {
      if (this.map.has(src.id)) return this.map.get(src.id);
      const type = MS.SourceTypes[src.type];
      if (!type) return null;
      const rt = type.create(src);
      this.map.set(src.id, rt);
      rt.start();
      return rt;
    },

    refresh(src) {
      const rt = this.map.get(src.id);
      if (rt && rt.update && rt.ready) {
        rt.src = src;
        rt.update();
        return rt;
      }
      this.dispose(src.id);
      return this.ensure(src);
    },

    dispose(id) {
      const rt = this.map.get(id);
      if (!rt) return;
      this.map.delete(id);
      MS.emit('runtime:disposed', id);
      rt.dispose();
    },

    sync() {
      const sources = MS.store.data.sources;
      for (const id of [...this.map.keys()]) if (!sources[id]) this.dispose(id);
      for (const src of Object.values(sources)) this.ensure(src);
    },
  };

  MS.on('sources:changed', () => MS.runtimes.sync());
})();
