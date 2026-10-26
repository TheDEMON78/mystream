'use strict';

(function () {
  const MS = window.MS;

  const LABELS = { record: "l'enregistrement", stream: 'le streaming' };

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

      // Le flux de streaming est réencodé par FFmpeg : on lui envoie une source de haute qualité.
      const vb = this.kind === 'record' ? settings.output.videoBitrate : Math.max(settings.stream.videoBitrate * 2, 8000);
      const ab = this.kind === 'record' ? settings.output.audioBitrate : settings.stream.audioBitrate;
      this.recorder = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: vb * 1000, audioBitsPerSecond: ab * 1000 });
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

  const outputs = {
    active: { record: null, stream: null },
    busy: { record: false, stream: false },
    _stream: null,

    init() {
      window.api.onOutputEnded((d) => this.onEnded(d));
      window.api.onOutputProgress((d) => {
        for (const o of Object.values(this.active)) if (o && o.id === d.id) o.progress = d;
      });
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
      return Boolean(this.active.record || this.active.stream);
    },

    async toggle(kind) {
      if (this.busy[kind]) return;
      if (this.active[kind]) await this.stop(kind);
      else await this.start(kind);
    },

    async start(kind) {
      this.busy[kind] = true;
      MS.emit('outputs:changed');
      const out = new Output(kind);
      try {
        await out.start(this.stream());
        this.active[kind] = out;
        if (kind === 'record') MS.toast(`Enregistrement démarré : ${out.file}`, 'info', 3500);
      } catch (e) {
        if (out.id) window.api.outputStop(out.id);
        MS.toast(`Impossible de démarrer ${LABELS[kind]} : ${e.message}`, 'error', 8000);
      } finally {
        this.busy[kind] = false;
        MS.emit('outputs:changed');
      }
    },

    async stop(kind) {
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
        else MS.toast(`Erreur d'enregistrement : ${res.error}`, 'error', 10000);
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
        MS.toast(`Arrêt de ${LABELS[kind]} : ${error || 'erreur inconnue'}`, 'error', 12000);
        MS.emit('outputs:changed');
      }
    },
  };

  MS.outputs = outputs;
})();
