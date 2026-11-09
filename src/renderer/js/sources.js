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

  /**
   * Images d'une capture vidéo (écran, fenêtre, jeu, caméra), lues
   * directement à la sortie du capteur : chaque image est utilisable dès
   * qu'elle est capturée, sans le tampon d'un lecteur <video> calé sur
   * l'affichage, et le compositeur est prévenu pour composer aussitôt.
   */
  class FrameFeed {
    constructor(rt, track) {
      this.rt = rt;
      this.interval = 0; // intervalle moyen entre deux images (ms)
      this.last = 0;
      this.first = new Promise((resolve) => (this.onFirst = resolve));
      if (typeof MediaStreamTrackProcessor === 'function') this.pump(track);
      else this.fallback(track);
    }
    async pump(track) {
      let reader;
      try {
        // Tampon d'une image : si l'interface prend du retard, on saute à la plus récente.
        reader = new MediaStreamTrackProcessor({ track, maxBufferSize: 1 }).readable.getReader();
      } catch {
        return this.fallback(track);
      }
      this.reader = reader;
      for (;;) {
        let r;
        try {
          r = await reader.read();
        } catch {
          break;
        }
        if (r.done) break;
        if (this.closed) {
          r.value.close();
          break;
        }
        if (this.frame) this.frame.close();
        this.frame = r.value;
        this.tick();
      }
    }
    /** Ancien moteur sans lecture directe : lecteur <video> et rappel à chaque image. */
    fallback(track) {
      const v = document.createElement('video');
      v.muted = true;
      v.playsInline = true;
      v.srcObject = new MediaStream([track]);
      v.play().catch(() => {});
      this.video = v;
      const step = () => {
        if (this.closed) return;
        this.tick();
        v.requestVideoFrameCallback(step);
      };
      if (v.requestVideoFrameCallback) v.requestVideoFrameCallback(step);
      else v.addEventListener('loadeddata', () => this.onFirst(), { once: true });
    }
    tick() {
      const now = performance.now();
      const dt = now - this.last;
      if (this.last && dt < 250) this.interval = this.interval ? this.interval * 0.9 + dt * 0.1 : dt;
      this.last = now;
      if (this.onFirst) {
        this.onFirst();
        this.onFirst = null;
      }
      MS.compositor.sourceFrame(this.rt, now);
    }
    /** Attend la première image (dimensions connues), 3 s au plus. */
    ready() {
      return Promise.race([this.first, new Promise((r) => setTimeout(r, 3000))]);
    }
    get width() {
      return this.frame ? this.frame.displayWidth : this.video ? this.video.videoWidth : 0;
    }
    get height() {
      return this.frame ? this.frame.displayHeight : this.video ? this.video.videoHeight : 0;
    }
    image() {
      if (this.frame) return this.frame;
      return this.video && this.video.readyState >= 2 ? this.video : null;
    }
    close() {
      this.closed = true;
      if (this.reader) this.reader.cancel().catch(() => {});
      if (this.frame) this.frame.close();
      this.frame = null;
      if (this.video) this.video.srcObject = null;
    }
  }
  MS.FrameFeed = FrameFeed;

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
        const vt = stream.getVideoTracks()[0];
        if (vt) {
          this.feed = new FrameFeed(this, vt);
          await this.feed.ready();
        }
        this.markReady();
      } catch (e) {
        this.fail(e);
      }
    }
    get width() {
      return this.feed ? this.feed.width : 0;
    }
    get height() {
      return this.feed ? this.feed.height : 0;
    }
    drawable() {
      return this.feed ? this.feed.image() : null;
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
      if (this.feed) this.feed.close();
      if (this.stream) this.stream.getTracks().forEach((t) => t.stop());
    }
  }

  class VideoCaptureRuntime extends StreamRuntime {
    async getStream() {
      const s = this.src.settings;
      const video = {};
      // Collection importée d'OBS : seul le nom du périphérique est connu.
      if (s.deviceLabel) {
        const devices = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'videoinput');
        if (!devices.some((d) => d.deviceId === s.deviceId)) {
          const want = s.deviceLabel.toLowerCase();
          const found = devices.find((d) => d.label.toLowerCase() === want) || devices.find((d) => d.label && (d.label.toLowerCase().includes(want) || want.includes(d.label.toLowerCase().replace(/\s*\(.*\)$/, ''))));
          if (found) s.deviceId = found.deviceId;
          else if (devices.length && !devices[0].label) throw new Error(`Autorisez l'accès à la caméra pour retrouver « ${s.deviceLabel} ».`);
          else throw new Error(`Caméra « ${s.deviceLabel} » introuvable : vérifiez qu'elle est branchée.`);
        }
      }
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

  /** Lecteur du son capturé par window-audio.exe (Windows). */
  function winAudioNode(rt, ctx, captureId) {
    if (!rt.pcm) {
      rt.pcm = new AudioWorkletNode(ctx, 'pcm-player', { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
      window.api.winAudioStart(rt.src.id, captureId);
    }
    return rt.pcm;
  }

  function pushPcm(rt, bytes) {
    if (!rt.pcm) return;
    const samples = new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength >> 2);
    rt.pcm.port.postMessage(samples, [samples.buffer]);
  }

  /**
   * Retrouve l'écran ou la fenêtre à capturer. Les identifiants changent à
   * chaque session : on cherche par nom exact, puis (fenêtres importées
   * d'OBS ou audio d'application) par mots du titre ou nom du programme, car
   * le titre change souvent (salon Discord, morceau en cours, version du jeu…).
   */
  MS.findCaptureTarget = function (list, s) {
    let target = list.find((x) => x.id === s.captureId);
    if (!target && s.captureName) target = list.find((x) => x.name === s.captureName && x.kind === s.captureKind);
    if (!target && s.captureKind === 'screen') target = list.find((x) => x.kind === 'screen');
    const m = s.windowMatch;
    if (!target && s.captureKind === 'window' && m) {
      const words = (t) => String(t || '').toLowerCase().normalize('NFKD').match(/[a-z0-9]{3,}/g) || [];
      const wanted = new Set([...words(m.title), ...words(String(m.exe || '').replace(/\.exe$/i, ''))]);
      const first = words(m.title)[0];
      let best = 0;
      for (const w of list.filter((x) => x.kind === 'window')) {
        const ws = words(w.name);
        let score = ws.filter((x) => wanted.has(x)).length;
        if (first && ws[0] === first) score += 2;
        if (score > best) {
          best = score;
          target = w;
        }
      }
    }
    return target || null;
  };

  const RETRY_WINDOW = 5000;

  class DisplayCaptureRuntime extends StreamRuntime {
    /** Son de la fenêtre capturée (ou du système pour un écran), sous Windows. */
    wantsAudio() {
      return Boolean(this.src.settings.captureAudio && MS.appInfo.windowAudio);
    }
    hasAudio() {
      return this.wantsAudio();
    }
    audioNode(ctx) {
      return winAudioNode(this, ctx, this.src.settings.captureId);
    }
    pushAudio(bytes) {
      pushPcm(this, bytes);
    }
    dispose() {
      clearTimeout(this.retry);
      if (this.pcm) window.api.winAudioStop(this.src.id);
      super.dispose();
    }
    /**
     * Comme OBS : une fenêtre absente (jeu pas encore lancé) ou fermée est
     * recherchée toutes les 5 s, et la capture reprend dès qu'elle revient.
     */
    fail(err) {
      super.fail(err);
      if (this.src.settings.captureKind !== 'window' || this.disposed) return;
      clearTimeout(this.retry);
      this.retry = setTimeout(() => {
        if (this.disposed) return;
        MS.runtimes.refresh(this.src);
      }, RETRY_WINDOW);
    }
    async getStream() {
      const s = this.src.settings;
      const list = await window.api.getCaptureSources(false);
      const target = MS.findCaptureTarget(list, s);
      if (!target) throw new Error(s.captureKind === 'window' ? `Fenêtre introuvable (${s.captureName || 'aucune'}) : nouvelle recherche toutes les 5 s` : 'Écran introuvable');
      s.captureId = target.id;
      if (target.kind === 'window') s.captureName = target.name;
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

  /**
   * Capture de jeu automatique, comme OBS en mode « n'importe quelle
   * application en plein écran » : aucun choix à faire, MyStream capture le
   * jeu ou l'application qui occupe tout l'écran (plein écran ou fenêtré sans
   * bordure), avec son son. Le jeu reste capturé pendant un alt-tab, jusqu'à
   * ce qu'il se ferme ou qu'une autre application passe en plein écran.
   */
  class GameCaptureRuntime extends BaseRuntime {
    async start() {
      if (!MS.appInfo.windowAudio) return this.fail(new Error('Nécessite Windows 10 version 2004 ou plus récent'));
      this.onGame = (g) => this.setGame(g);
      this.onForeground = (fg) => this.setForeground(fg);
      MS.on('game:changed', this.onGame);
      MS.on('game:foreground', this.onForeground);
      window.api.gameWatch(this.src.id, true);
      this.markReady();
      this.setGame(await window.api.gameCurrent());
    }
    get width() {
      return this.frozen ? this.frozen.width : this.feed ? this.feed.width : 0;
    }
    get height() {
      return this.frozen ? this.frozen.height : this.feed ? this.feed.height : 0;
    }
    drawable() {
      if (this.frozen) return this.frozen;
      return this.feed ? this.feed.image() : null;
    }
    /** Capture de l'écran du jeu (le plus fluide), ou de sa fenêtre si l'écran est introuvable ou si on le demande. */
    sourceOf(g) {
      return this.src.settings.captureMode !== 'window' && g.screenId ? g.screenId : g.id;
    }
    /**
     * Alt+Tab pendant une capture d'écran : on garde la dernière image du jeu
     * au lieu de montrer le bureau ou une autre application.
     */
    setForeground(fg) {
      const img = this.feed && this.feed.image();
      if (fg || !img || !this.game || this.captured !== this.game.screenId) {
        this.frozen = null;
        return;
      }
      const c = document.createElement('canvas');
      c.width = this.feed.width;
      c.height = this.feed.height;
      c.getContext('2d').drawImage(img, 0, 0);
      this.frozen = c;
    }
    /** Nom du jeu capturé (affiché dans la liste des sources). */
    get status() {
      return this.game ? `${this.game.title || this.game.exe}` : 'En attente d’un jeu en plein écran';
    }
    async setGame(g) {
      const key = (x) => (x ? `${x.id}|${x.screenId}` : '');
      if (this.disposed || key(g) === key(this.game)) return;
      this.game = g || null;
      this.frozen = null;
      const token = (this.token = (this.token || 0) + 1);
      this.stopVideo();
      this.syncAudio();
      MS.emit('runtime:ready', this);
      if (!g) return;
      const { width, height, fps } = MS.store.settings.video;
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: false,
          video: { mandatory: { chromeMediaSource: 'desktop', chromeMediaSourceId: this.sourceOf(g), maxWidth: width, maxHeight: height, maxFrameRate: fps } },
        });
        if (token !== this.token || this.disposed) return stream.getTracks().forEach((t) => t.stop());
        this.captured = this.sourceOf(g);
        this.stream = stream;
        this.feed = new FrameFeed(this, stream.getVideoTracks()[0]);
        await this.feed.ready();
        if (token !== this.token || this.disposed) return;
        this.error = null;
        MS.emit('runtime:ready', this);
      } catch (e) {
        if (token === this.token) this.fail(new Error(`Capture de « ${g.title || g.exe} » impossible : ${e.message}`));
      }
    }
    stopVideo() {
      if (this.feed) this.feed.close();
      if (this.stream) this.stream.getTracks().forEach((t) => t.stop());
      this.stream = null;
      this.feed = null;
    }
    hasAudio() {
      return Boolean(this.src.settings.captureAudio && MS.appInfo.windowAudio);
    }
    /** Le lecteur reste en place ; seule la capture du son suit le jeu. */
    audioNode(ctx) {
      if (!this.pcm) this.pcm = new AudioWorkletNode(ctx, 'pcm-player', { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
      this.syncAudio();
      return this.pcm;
    }
    syncAudio() {
      if (!this.pcm) return;
      if (this.game && this.hasAudio()) window.api.winAudioStart(this.src.id, this.game.id);
      else window.api.winAudioStop(this.src.id);
    }
    pushAudio(bytes) {
      pushPcm(this, bytes);
    }
    /** Le jeu qui se ferme coupe son son : ce n'est pas une erreur à signaler. */
    onAudioError() {
      return true;
    }
    dispose() {
      super.dispose();
      if (this.onGame) MS.off('game:changed', this.onGame);
      if (this.onForeground) MS.off('game:foreground', this.onForeground);
      window.api.gameWatch(this.src.id, false);
      this.stopVideo();
      if (this.pcm) window.api.winAudioStop(this.src.id);
    }
  }

  /**
   * Son d'une application seule (Discord, navigateur, lecteur de musique…),
   * sans image : l'équivalent de « Capture audio d'application » d'OBS.
   */
  class AppAudioRuntime extends BaseRuntime {
    async start() {
      if (!MS.appInfo.windowAudio) return this.fail(new Error('Nécessite Windows 10 version 2004 ou plus récent'));
      const s = this.src.settings;
      // Programme connu : capturé par son nom, avec ou sans fenêtre ; le module natif attend son lancement.
      if (s.exe && s.exe.trim()) {
        s.captureId = `exe:${s.exe.trim()}`;
        return this.markReady();
      }
      try {
        const list = await window.api.getCaptureSources(false);
        const target = MS.findCaptureTarget(list, { ...s, captureKind: 'window' });
        if (!target) throw new Error(`Application introuvable (${s.captureName || 'aucune'}) : nouvelle recherche toutes les 5 s`);
        s.captureId = target.id;
        s.captureName = target.name;
        this.markReady();
      } catch (e) {
        this.fail(e);
        clearTimeout(this.retry);
        this.retry = setTimeout(() => !this.disposed && MS.runtimes.refresh(this.src), RETRY_WINDOW);
      }
    }
    onAudioState({ state, value }) {
      const s = this.src.settings;
      if (state === 'exe' && value && !s.exe) {
        // Ancienne source liée à une fenêtre : on retient le programme pour le retrouver même sans fenêtre.
        s.exe = value;
        MS.store.save();
        return MS.runtimes.refresh(this.src);
      }
      if (state === 'waiting') this.status = `${s.exe} n'est pas lancé : capture dès son ouverture`;
      else if (state === 'capturing') this.status = '';
      else return;
      MS.emit('runtime:status', this);
    }
    hasAudio() {
      return this.ready;
    }
    audioNode(ctx) {
      return winAudioNode(this, ctx, this.src.settings.captureId);
    }
    pushAudio(bytes) {
      pushPcm(this, bytes);
    }
    dispose() {
      clearTimeout(this.retry);
      if (this.pcm) window.api.winAudioStop(this.src.id);
      super.dispose();
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
    /** Sous Windows : tout le son du PC sauf MyStream (pas d'écho du retour casque). */
    async start() {
      if (!MS.appInfo.windowAudio) return super.start();
      this.markReady();
    }
    hasAudio() {
      return MS.appInfo.windowAudio ? true : super.hasAudio();
    }
    audioNode(ctx) {
      return MS.appInfo.windowAudio ? winAudioNode(this, ctx, 'screen:system') : super.audioNode(ctx);
    }
    pushAudio(bytes) {
      pushPcm(this, bytes);
    }
    dispose() {
      if (this.pcm) window.api.winAudioStop(this.src.id);
      super.dispose();
    }
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
      if (MS.isGif(this.src.settings.file) && 'ImageDecoder' in window) {
        try {
          this.gif = await new MS.GifPlayer(MS.fileUrl(this.src.settings.file)).load();
          if (this.disposed) return this.gif.close();
          return this.markReady();
        } catch (e) {
          return this.fail(new Error(`GIF illisible : ${e.message}`));
        }
      }
      const img = new Image();
      img.onload = () => this.markReady();
      img.onerror = () => this.fail(new Error('Image illisible'));
      this.img = img;
      if (this.src.settings.file) img.src = MS.fileUrl(this.src.settings.file);
    }
    get width() {
      if (this.gif) return this.gif.width;
      return this.img ? this.img.naturalWidth : 0;
    }
    get height() {
      if (this.gif) return this.gif.height;
      return this.img ? this.img.naturalHeight : 0;
    }
    drawable() {
      if (!this.ready) return null;
      return this.gif ? this.gif.current(true) : this.img;
    }
    dispose() {
      super.dispose();
      if (this.gif) this.gif.close();
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
      // Une emote qui finit de charger change la largeur des messages : mise en page à refaire.
      this.onImage = () => {
        this.layouts = new WeakMap();
        this.onMessage();
      };
      MS.on('chat:message', this.onMessage);
      MS.on('chat:cleared', this.onMessage);
      MS.on('chat:image', this.onImage);
      // Messages éphémères : on ne redessine que pendant un fondu de disparition.
      this.timer = setInterval(() => this.src.settings.lifetime > 0 && Date.now() >= this.nextFade && this.update(), 100);
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
      this.nextFade = Infinity;
      // Mise en page de chaque message gardée tant que l'apparence ne change pas.
      const key = [W, size, s.font, s.showIcons, s.showBadges, s.textColor, s.nameColor].join('|');
      if (key !== this.layoutKey || !this.layouts) {
        this.layoutKey = key;
        this.layouts = new WeakMap();
      }

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
          this.nextFade = Math.min(this.nextFade, m.at + life - 1000);
        }
        let rows = this.layouts.get(m);
        if (!rows) this.layouts.set(m, (rows = this.layout(ctx, m, W - pad * 2, size, font)));
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
        tokens.push({ kind: 'text', text: `[${MS.platformName(m.platform)}]`, color: '#9aa0b4', bold: true });
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
        ctx.fill(path2d(MS.PLATFORM_ICONS[t.platform]));
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
        ctx.stroke(path2d(ic.d));
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
      MS.off('chat:message', this.onMessage);
      MS.off('chat:cleared', this.onMessage);
      MS.off('chat:image', this.onImage);
    }
  }

  /** Tracés des logos et badges, analysés une seule fois. */
  const paths2d = new Map();
  function path2d(d) {
    let p = paths2d.get(d);
    if (!p) paths2d.set(d, (p = new Path2D(d)));
    return p;
  }

  /** Page web rendue hors écran par le processus principal. */
  class BrowserRuntime extends BaseRuntime {
    async start() {
      this.canvas = document.createElement('canvas');
      this.ctx = this.canvas.getContext('2d');
      this.audioOn = Boolean(this.src.settings.audio);
      this.signature = this.sig();
      window.api.browserCreate(this.src.id, { ...this.src.settings, maxFps: MS.store.settings.video.fps });
    }
    sig() {
      const s = this.src.settings;
      return JSON.stringify([s.url, s.width, s.height, s.fps, s.css, s.audio]);
    }
    update() {
      if (this.sig() === this.signature) return;
      // Son activé ou coupé : la voie du mixeur est à créer ou à retirer.
      if (Boolean(this.src.settings.audio) !== this.audioOn) {
        MS.runtimes.dispose(this.src.id);
        MS.runtimes.ensure(this.src);
        return;
      }
      this.signature = this.sig();
      window.api.browserCreate(this.src.id, { ...this.src.settings, maxFps: MS.store.settings.video.fps });
    }
    hasAudio() {
      return this.audioOn;
    }
    /** Son de la page, capturé dans la page et reçu comme celui d'une fenêtre. */
    audioNode(ctx) {
      if (!this.pcm) this.pcm = new AudioWorkletNode(ctx, 'pcm-player', { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2] });
      return this.pcm;
    }
    pushAudio(bytes) {
      pushPcm(this, bytes);
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
  window.api.onGameChanged((g) => MS.emit('game:changed', g));
  window.api.onGameForeground((fg) => MS.emit('game:foreground', fg));
  window.api.onWinAudioData(({ id, data }) => {
    const rt = MS.runtimes.get(id);
    if (rt && rt.pushAudio) rt.pushAudio(data);
  });
  window.api.onWinAudioState((st) => {
    const rt = MS.runtimes.get(st.id);
    if (rt && rt.onAudioState) rt.onAudioState(st);
  });
  window.api.onWinAudioError(({ id, error }) => {
    const rt = MS.runtimes.get(id);
    if (!rt || (rt.onAudioError && rt.onAudioError(error))) return;
    MS.toast(`Son de « ${rt.src.name} » : ${error}`, 'error', 8000);
  });

  window.api.onBrowserAudio(({ id, data }) => {
    const rt = MS.runtimes.get(id);
    if (rt && rt.pushAudio) rt.pushAudio(data);
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
    game_capture: {
      label: 'Capture de jeu (automatique)',
      glyph: '🎮',
      defaults: () => ({ captureAudio: true, captureMode: 'screen' }),
      create: (src) => new GameCaptureRuntime(src),
      fields: () => [
        { type: 'info', text: "Aucun réglage nécessaire : MyStream détecte le jeu ou l'application affiché en plein écran ou en fenêtré sans bordure, et capture aussi son son. Lancez votre jeu, il apparaît automatiquement. Pendant un Alt+Tab, la dernière image du jeu reste affichée : votre bureau n'apparaît pas dans le stream." },
        { key: 'captureAudio', type: 'checkbox', label: 'Capturer le son du jeu' },
        {
          key: 'captureMode',
          type: 'select',
          label: 'Méthode de capture',
          options: [
            { value: 'screen', label: 'Écran du jeu : la plus fluide, sans ralentir le jeu (recommandée)' },
            { value: 'window', label: 'Fenêtre du jeu : plus lente, pour un jeu qui ne couvre pas tout l’écran' },
          ],
        },
      ],
    },
    display_capture: {
      label: "Capture d'écran / fenêtre",
      glyph: '🖥',
      defaults: () => ({ captureId: '', captureName: '', captureKind: 'screen', captureAudio: true }),
      create: (src) => new DisplayCaptureRuntime(src),
      fields: () => [
        { key: 'capture', type: 'capture', label: 'Écran ou fenêtre' },
        MS.appInfo.windowAudio
          ? { key: 'captureAudio', type: 'checkbox', label: 'Capturer le son (fenêtre : son de l’application ; écran : son du PC, hors MyStream)' }
          : { type: 'info', text: 'La capture du son d’une fenêtre nécessite Windows 10 version 2004 ou plus récent.' },
      ],
    },
    video_capture: {
      label: 'Périphérique de capture vidéo',
      glyph: '📷',
      defaults: () => ({ deviceId: '', resolution: '1920x1080', fps: '60' }),
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
      fields: () => [{ type: 'info', text: 'Capture tout le son du PC. Disponible uniquement sous Windows.' }],
    },
    app_audio: {
      label: "Capture audio d'application",
      glyph: '🎧',
      audioOnly: true,
      defaults: () => ({ exe: '', captureId: '', captureName: '', captureKind: 'window' }),
      create: (src) => new AppAudioRuntime(src),
      fields: async () => {
        const apps = await window.api.audioApps().catch(() => []);
        return [
          { type: 'info', text: "Seul le son de ce programme est capturé (Discord, navigateur, Spotify…), sans l'image. Il est capturé même réduit dans la zone de notification, sans fenêtre ouverte. S'il est fermé, MyStream attend son lancement et le retrouve s'il redémarre." },
          { key: 'exe', type: 'program', label: 'Programme', apps },
          { type: 'info', text: 'Programme absent de la liste ? Lancez-le puis rouvrez ces propriétés, ou choisissez « Autre programme… » et saisissez le nom de son fichier (ex. Discord.exe, Spotify.exe).' },
        ];
      },
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
        { key: 'nameColor', type: 'color', label: 'Couleur des pseudos (si la plateforme n’en fournit pas)' },
        { key: 'bgColor', type: 'color', label: 'Couleur de fond' },
        { key: 'bgOpacity', type: 'range', label: 'Opacité du fond (%)', min: 0, max: 100 },
        { key: 'lifetime', type: 'number', label: 'Masquer les messages après (secondes, 0 = jamais)', min: 0, max: 3600 },
        { key: 'showIcons', type: 'checkbox', label: 'Logo de la plateforme' },
        { key: 'showBadges', type: 'checkbox', label: 'Badges (modérateur, abonné…)' },
        { key: 'showSystem', type: 'checkbox', label: 'Messages système' },
        { key: 'shadow', type: 'checkbox', label: 'Contour du texte' },
      ],
    },
    browser: {
      label: 'Navigateur',
      glyph: '🌐',
      // 30 i/s suffisent aux alertes et widgets, et coûtent deux fois moins que 60.
      defaults: () => ({ url: '', width: 800, height: 600, fps: 30, css: MS.appInfo.defaultBrowserCss, audio: true }),
      create: (src) => new BrowserRuntime(src),
      fields: () => [
        { key: 'url', type: 'text', label: 'URL (ou chemin d’un fichier HTML local)', placeholder: 'https://…' },
        { key: 'width', type: 'number', label: 'Largeur', min: 16, max: 4096 },
        { key: 'height', type: 'number', label: 'Hauteur', min: 16, max: 4096 },
        { key: 'fps', type: 'number', label: 'Images par seconde', min: 1, max: 60 },
        { key: 'audio', type: 'checkbox', label: 'Envoyer le son de la page dans le stream (réglable dans le mixeur)' },
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
