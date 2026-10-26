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
            maxWidth: Math.max(width, 3840),
            maxHeight: Math.max(height, 2160),
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
