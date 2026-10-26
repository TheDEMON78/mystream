'use strict';

(function () {
  const MS = window.MS;

  const audio = {
    ctx: null,
    dest: null,
    channels: new Map(),

    init() {
      this.ctx = new AudioContext({ latencyHint: 'interactive', sampleRate: 48000 });
      this.dest = this.ctx.createMediaStreamDestination();
      this.dest.channelCount = 2;
      // Silence permanent : sans aucune source audio, la piste de sortie ne
      // produirait aucun échantillon et l'encodeur attendrait indéfiniment.
      const silence = this.ctx.createConstantSource();
      silence.offset.value = 0;
      silence.connect(this.dest);
      silence.start();

      MS.on('runtime:ready', (rt) => {
        if (rt.hasAudio()) this.attach(rt);
      });
      MS.on('runtime:disposed', (id) => this.detach(id));
      for (const evt of ['scene:switched', 'items:changed', 'sources:changed']) MS.on(evt, () => this.updateGains());
      document.addEventListener('pointerdown', () => this.ctx.state === 'suspended' && this.ctx.resume(), true);
    },

    attach(rt) {
      const id = rt.src.id;
      this.detach(id);
      let node;
      try {
        node = rt.audioNode(this.ctx);
      } catch (e) {
        console.warn('Audio indisponible pour', rt.src.name, e);
        return;
      }
      if (!node) return;
      const ctx = this.ctx;
      const gain = ctx.createGain();
      const monitor = ctx.createGain();
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 1024;
      gain.gain.value = 0;
      monitor.gain.value = 0;
      node.connect(gain);
      gain.connect(analyser);
      gain.connect(this.dest);
      gain.connect(monitor);
      monitor.connect(ctx.destination);
      this.channels.set(id, { id, node, gain, monitor, analyser, buf: new Float32Array(analyser.fftSize), peak: -Infinity });
      this.updateGains();
      MS.emit('mixer:changed');
    },

    detach(id) {
      const ch = this.channels.get(id);
      if (!ch) return;
      for (const n of [ch.node, ch.gain, ch.monitor]) {
        try {
          n.disconnect();
        } catch {
          /* déjà déconnecté */
        }
      }
      this.channels.delete(id);
      MS.emit('mixer:changed');
    },

    /** Sources audibles : les sources globales et celles visibles dans la scène active. */
    activeSet() {
      const set = new Set();
      for (const s of Object.values(MS.store.data.sources)) if (s.global) set.add(s.id);
      const scene = MS.store.currentScene();
      if (scene) for (const it of scene.items) if (it.visible) set.add(it.sourceId);
      return set;
    },

    /** Sources affichées dans le mixeur (comme OBS : globales + sources de la scène). */
    mixerSources() {
      const ids = new Set();
      for (const s of Object.values(MS.store.data.sources)) if (s.global) ids.add(s.id);
      const scene = MS.store.currentScene();
      if (scene) for (const it of scene.items) ids.add(it.sourceId);
      return [...ids].filter((id) => this.channels.has(id)).map((id) => MS.store.source(id)).filter(Boolean);
    },

    updateGains() {
      if (!this.ctx) return;
      const active = this.activeSet();
      const t = this.ctx.currentTime;
      for (const [id, ch] of this.channels) {
        const s = MS.store.source(id);
        if (!s) continue;
        const g = active.has(id) && !s.muted ? s.volume : 0;
        ch.gain.gain.setTargetAtTime(g, t, 0.015);
        ch.monitor.gain.setTargetAtTime(s.monitor ? 1 : 0, t, 0.015);
      }
    },

    /** Niveau crête en dBFS (après le fader). */
    level(id) {
      const ch = this.channels.get(id);
      if (!ch) return -Infinity;
      ch.analyser.getFloatTimeDomainData(ch.buf);
      let peak = 0;
      for (let i = 0; i < ch.buf.length; i++) {
        const v = Math.abs(ch.buf[i]);
        if (v > peak) peak = v;
      }
      const db = peak > 0 ? 20 * Math.log10(peak) : -Infinity;
      // Retombée progressive de l'indicateur, comme une vraie VU-mètre.
      ch.peak = db > ch.peak ? db : Math.max(db, ch.peak - 1.2);
      return ch.peak;
    },

    /** Le fader utilise une courbe cubique : position 0..1 -> gain linéaire. */
    faderToGain: (p) => p * p * p,
    gainToFader: (g) => Math.cbrt(g),
  };

  MS.audio = audio;

  /* Sources audio globales (Mic/Aux et Audio du bureau) ---------------- */
  MS.syncGlobalAudio = function () {
    const store = MS.store;
    const cfg = store.settings.audio;
    const mic = store.source('global_mic');
    if (cfg.micDevice === 'disabled') {
      if (mic) store.deleteSource('global_mic');
    } else if (!mic) {
      store.createSource('audio_input', 'Mic/Aux', { deviceId: cfg.micDevice, noiseSuppression: false }, { id: 'global_mic', global: true });
    } else if (mic.settings.deviceId !== cfg.micDevice) {
      mic.settings.deviceId = cfg.micDevice;
      store.save();
      MS.runtimes.refresh(mic);
    }

    const desk = store.source('global_desktop');
    if (cfg.desktopAudio && !desk) store.createSource('desktop_audio', 'Audio du bureau', {}, { id: 'global_desktop', global: true });
    else if (!cfg.desktopAudio && desk) store.deleteSource('global_desktop');
  };
})();
