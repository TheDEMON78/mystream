'use strict';

(function () {
  const MS = window.MS;

  const LABELS = { record: "l'enregistrement", stream: 'le streaming' };
  const NAMES = { record: "L'enregistrement", stream: 'Le streaming' };

  function pickMime(kind) {
    const fmt = MS.store.settings.output.format;
    const h264 = ['video/x-matroska;codecs=avc1,opus', 'video/webm;codecs=h264,opus'];
    const vp = ['video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm'];
    const order = kind === 'record' && fmt === 'webm' ? vp : [...h264, ...vp];
    return order.find((m) => MediaRecorder.isTypeSupported(m)) || '';
  }

  class Output {
    constructor(kind) {
      this.kind = kind;
      this.id = null;
      this.recorder = null;
      this.startedAt = 0;
      this.progress = null;
      this.queue = Promise.resolve();
    }

    async start(stream) {
      const settings = MS.store.settings;
      const mime = pickMime(this.kind);
      const res = await window.api.outputStart({ kind: this.kind, mime, settings });
      if (!res.ok) throw new Error(res.error);
      this.id = res.id;
      this.file = res.file;
      this.encoder = res.encoder;

      // Le flux de streaming est réencodé par FFmpeg : on lui envoie une source
      // trois fois plus riche que le débit final, pour que ce second encodage
      // ne dégrade presque pas l'image (le flux reste local, sa taille ne coûte rien).
      const vb = this.kind === 'record' ? settings.output.videoBitrate : MS.clamp(settings.stream.videoBitrate * 3, 12000, 60000);
      const ab = this.kind === 'record' ? settings.output.audioBitrate : settings.stream.audioBitrate;
      // Une image clé toutes les 2 s : indispensable pour que l'enregistrement
      // reste lisible après une coupure et pour la navigation dans la vidéo.
      this.recorder = new MediaRecorder(stream, {
        mimeType: mime,
        videoBitsPerSecond: vb * 1000,
        audioBitsPerSecond: ab * 1000,
        videoKeyFrameIntervalDuration: 2000,
      });
      this.recorder.ondataavailable = (e) => {
        if (!e.data.size) return;
        this.queue = this.queue.then(() => e.data.arrayBuffer()).then((buf) => window.api.outputChunk(this.id, buf));
      };
      this.recorder.start(250);
      this.startedAt = performance.now();
    }

    stop() {
      return new Promise((resolve) => {
        const finish = async () => {
          await this.queue;
          resolve(await window.api.outputStop(this.id));
        };
        if (this.recorder && this.recorder.state !== 'inactive') {
          this.recorder.onstop = finish;
          this.recorder.stop();
        } else {
          finish();
        }
      });
    }
  }

  const RECONNECT_DELAY = 10; // secondes, comme OBS
  const RECONNECT_RETRIES = 25;
  const DISK_WARN = 5 * 1024 ** 3;
  const DISK_STOP = 300 * 1024 ** 2;

  const outputs = {
    active: { record: null, stream: null },
    busy: { record: false, stream: false },
    reconnect: null,
    health: { level: 'idle', message: '' },
    _stream: null,

    init() {
      window.api.onOutputEnded((d) => this.onEnded(d));
      window.api.onOutputProgress((d) => {
        for (const o of Object.values(this.active)) if (o && o.id === d.id) o.progress = d;
      });
      setInterval(() => this.checkHealth(), 2000);
      setInterval(() => this.checkDisk(), 30000);
    },

    /** Flux composite : image du canvas + mixage audio. */
    stream() {
      if (!this._stream) {
        const v = MS.compositor.canvas.captureStream(MS.store.settings.video.fps).getVideoTracks()[0];
        const a = MS.audio.dest.stream.getAudioTracks()[0];
        this._stream = new MediaStream([v, a].filter(Boolean));
      }
      return this._stream;
    },

    resetStream() {
      if (this.isAnyActive()) return;
      if (this._stream) this._stream.getVideoTracks().forEach((t) => t.stop());
      this._stream = null;
    },

    isAnyActive() {
      return Boolean(this.active.record || this.active.stream || this.reconnect);
    },

    async toggle(kind) {
      if (this.busy[kind]) return;
      if (this.active[kind] || (kind === 'stream' && this.reconnect)) await this.stop(kind);
      else await this.start(kind);
    },

    async start(kind) {
      this.busy[kind] = true;
      MS.emit('outputs:changed');
      try {
        if (kind === 'record') {
          const free = await window.api.diskFree(MS.store.settings.output.recordPath);
          if (free != null && free < DISK_STOP) throw new Error(`espace disque insuffisant (${formatBytes(free)} disponibles)`);
          if (free != null && free < DISK_WARN) MS.toast(`Attention : il ne reste que ${formatBytes(free)} sur le disque d'enregistrement.`, 'error', 8000);
        }
        const out = new Output(kind);
        try {
          await out.start(this.stream());
        } catch (e) {
          if (out.id) window.api.outputStop(out.id);
          throw e;
        }
        this.active[kind] = out;
        if (kind === 'record') MS.toast(`Enregistrement démarré : ${out.file}`, 'info', 3500);
        else {
          const enc = (MS.encoders.find((e) => e.id === out.encoder) || { label: out.encoder }).label;
          MS.toast(`Streaming démarré (encodeur : ${enc}).`, 'info', 3500);
        }
      } catch (e) {
        MS.toast(`Impossible de démarrer ${LABELS[kind]} : ${e.message}`, 'error', 8000);
      } finally {
        this.busy[kind] = false;
        MS.emit('outputs:changed');
      }
    },

    async stop(kind) {
      if (kind === 'stream' && this.reconnect) {
        clearTimeout(this.reconnect.timer);
        this.reconnect = null;
        MS.emit('outputs:changed');
      }
      const out = this.active[kind];
      if (!out) return;
      this.busy[kind] = true;
      MS.emit('outputs:changed');
      const res = await out.stop();
      this.active[kind] = null;
      this.busy[kind] = false;
      MS.emit('outputs:changed');
      if (kind === 'record') {
        if (res.ok) MS.toast(`Enregistrement sauvegardé : ${res.file}`, 'success', 6000);
        else MS.toast(`L'enregistrement a échoué : ${res.error}`, 'error', 10000);
      }
    },

    async stopAll() {
      await Promise.all(Object.keys(this.active).map((k) => this.stop(k)));
    },

    onEnded({ id, error }) {
      for (const [kind, out] of Object.entries(this.active)) {
        if (!out || out.id !== id) continue;
        if (out.recorder && out.recorder.state !== 'inactive') out.recorder.stop();
        window.api.outputStop(id);
        this.active[kind] = null;
        window.api.log('error', `${kind} interrompu : ${error}`);
        if (kind === 'stream') this.scheduleReconnect(out, error);
        else MS.toast(`${NAMES[kind]} s'est arrêté : ${error || 'erreur inconnue'}`, 'error', 12000);
        MS.emit('outputs:changed');
      }
    },

    /**
     * Reconnexion automatique après une coupure (réseau, serveur…), comme
     * OBS : nouvel essai toutes les 10 s, le chronomètre du live continue.
     */
    scheduleReconnect(prev, error) {
      // Une connexion restée stable plus d'une minute remet le compteur à zéro.
      const stable = prev.startedAt && performance.now() - (prev.connectedAt || prev.startedAt) > 60000;
      const attempt = stable ? 1 : (prev.attempt || 0) + 1;
      const liveStart = prev.liveStart || prev.startedAt || performance.now();
      if (attempt > RECONNECT_RETRIES) {
        this.reconnect = null;
        MS.toast(`Streaming arrêté après ${RECONNECT_RETRIES} tentatives de reconnexion : ${error || 'erreur inconnue'}`, 'error', 15000);
        return;
      }
      MS.toast(`Connexion au serveur perdue (${error || 'erreur inconnue'}). Nouvel essai dans ${RECONNECT_DELAY} s (${attempt} sur ${RECONNECT_RETRIES})…`, 'error', 8000);
      const r = { attempt, liveStart, error };
      r.timer = setTimeout(async () => {
        if (this.reconnect !== r) return;
        const out = new Output('stream');
        out.attempt = attempt;
        out.liveStart = liveStart;
        try {
          await out.start(this.stream());
          if (this.reconnect !== r) {
            out.stop();
            return;
          }
          this.reconnect = null;
          this.active.stream = out;
          out.connectedAt = performance.now();
          MS.toast('Reconnecté au serveur de streaming.', 'success', 5000);
        } catch (e) {
          if (out.id) window.api.outputStop(out.id);
          if (this.reconnect === r) this.scheduleReconnect({ attempt, liveStart }, e.message);
        }
        MS.emit('outputs:changed');
      }, RECONNECT_DELAY * 1000);
      this.reconnect = r;
    },

    /** Surveille l'encodeur et le rendu pour expliquer clairement les images perdues. */
    checkHealth() {
      const stream = this.active.stream;
      const stats = MS.compositor.stats;
      const lagged = stats.lagged - (this.lastLagged ?? stats.lagged);
      this.lastLagged = stats.lagged;
      let level = 'good';
      let message = 'Tout fonctionne normalement.';
      const fps = MS.store.settings.video.fps;
      // Deux relevés consécutifs au-delà de 10 % : on ignore les à-coups ponctuels (démarrage…).
      this.lagStreak = lagged > fps * 0.2 ? (this.lagStreak || 0) + 1 : 0;
      if (this.lagStreak >= 2) {
        level = 'warn';
        message = `Rendu surchargé (${lagged} images manquées en 2 s) : réduisez la résolution ou le nombre d'images par seconde, ou retirez des sources lourdes (pages web animées, captures multiples).`;
      }
      if (stream && stream.progress && stream.progress.speed) {
        stream.slow = stream.progress.speed < 0.93 ? (stream.slow || 0) + 1 : 0;
        if (stream.slow >= 3) {
          level = 'bad';
          message = `Encodeur ou connexion saturés (vitesse ${stream.progress.speed.toFixed(2)}×) : baissez le débit vidéo, choisissez un encodeur matériel (NVENC…) ou vérifiez votre connexion.`;
        }
      }
      if (this.reconnect) {
        level = 'bad';
        message = `Reconnexion en cours (essai ${this.reconnect.attempt} sur ${RECONNECT_RETRIES})${this.reconnect.error ? ` : ${this.reconnect.error}` : '.'}`;
      }
      if (!this.isAnyActive() && level === 'good') level = 'idle';
      const changed = level !== this.health.level || message !== this.health.message;
      this.health = { level, message };
      if (changed) MS.emit('health:changed', this.health);
      // Prévient une seule fois par problème (toutes les 2 minutes au plus).
      if ((level === 'warn' || level === 'bad') && this.isAnyActive() && !this.reconnect && performance.now() - (this.lastHealthToast || -1e9) > 120000) {
        this.lastHealthToast = performance.now();
        MS.toast(message, 'error', 10000);
      }
    },

    async checkDisk() {
      if (!this.active.record) return;
      const free = await window.api.diskFree(MS.store.settings.output.recordPath);
      if (free == null) return;
      if (free < DISK_STOP) {
        MS.toast(`Disque presque plein (${formatBytes(free)} disponibles) : l'enregistrement a été arrêté pour protéger le fichier.`, 'error', 15000);
        this.stop('record');
      } else if (free < DISK_WARN && !this.diskWarned) {
        this.diskWarned = true;
        MS.toast(`Attention : il ne reste que ${formatBytes(free)} sur le disque d'enregistrement.`, 'error', 10000);
      }
    },
  };

  function formatBytes(n) {
    return n >= 1024 ** 3 ? `${(n / 1024 ** 3).toFixed(1)} Go` : `${Math.round(n / 1024 ** 2)} Mo`;
  }

  MS.outputs = outputs;
})();
