'use strict';

(function () {
  const MS = window.MS;

  MS.audioFilterDefaults = () => ({
    gain: 0,
    gate: false,
    gateThreshold: -45,
    gateRelease: 150,
    compressor: false,
    compThreshold: -18,
    compRatio: 4,
    compMakeup: 4,
    limiter: false,
  });

  const dbToGain = (db) => Math.pow(10, db / 20);

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
      this.gateReady = this.ctx.audioWorklet
        .addModule('js/gate-worklet.js')
        .then(() => true)
        .catch((e) => {
          console.warn('Noise gate indisponible', e);
          return false;
        });
      this.pcmReady = this.ctx.audioWorklet.addModule('js/pcm-worklet.js').catch((e) => console.warn('Lecteur PCM indisponible', e));
    },

    /**
     * Chaîne de filtres d'une source : gain → noise gate → compresseur →
     * limiteur, puis le fader. Reconstruite à chaque modification.
     */
    async buildChain(ch) {
      // Plusieurs réglages rapides (curseurs) : seule la dernière reconstruction compte.
      const build = (ch.build = (ch.build || 0) + 1);
      const gateOk = await this.gateReady;
      if (ch.build !== build || this.channels.get(ch.id) !== ch) return;
      const src = MS.store.source(ch.id);
      const f = { ...MS.audioFilterDefaults(), ...((src && src.audioFilters) || {}) };
      const ctx = this.ctx;
      try {
        ch.node.disconnect();
      } catch {
        /* pas encore connecté */
      }
      for (const n of ch.chain || []) n.disconnect();
      const chain = [];
      if (f.gain) {
        const g = ctx.createGain();
        g.gain.value = dbToGain(f.gain);
        chain.push(g);
      }
      if (f.gate && gateOk) {
        const gate = new AudioWorkletNode(ctx, 'noise-gate', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2] });
        gate.parameters.get('threshold').value = f.gateThreshold;
        gate.parameters.get('release').value = f.gateRelease;
        chain.push(gate);
      }
      if (f.compressor) {
        const c = ctx.createDynamicsCompressor();
        c.threshold.value = f.compThreshold;
        c.ratio.value = f.compRatio;
        c.knee.value = 6;
        c.attack.value = 0.006;
        c.release.value = 0.1;
        const makeup = ctx.createGain();
        makeup.gain.value = dbToGain(f.compMakeup);
        chain.push(c, makeup);
      }
      if (f.limiter) {
        const l = ctx.createDynamicsCompressor();
        l.threshold.value = -1;
        l.ratio.value = 20;
        l.knee.value = 0;
        l.attack.value = 0.001;
        l.release.value = 0.05;
        chain.push(l);
      }
      ch.chain = chain;
      let prev = ch.node;
      for (const n of chain) {
        prev.connect(n);
        prev = n;
      }
      prev.connect(ch.gain);
    },

    refreshFilters(id) {
      const ch = this.channels.get(id);
      if (ch) this.buildChain(ch);
    },

    async attach(rt) {
      // Le lecteur du son de fenêtre est un AudioWorklet : il doit être chargé avant.
      await this.pcmReady;
      if (rt.disposed) return;
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
      gain.connect(analyser);
      gain.connect(this.dest);
      gain.connect(monitor);
      monitor.connect(ctx.destination);
      const ch = { id, node, gain, monitor, analyser, chain: [], buf: new Float32Array(analyser.fftSize), peak: -Infinity };
      this.channels.set(id, ch);
      this.buildChain(ch);
      this.updateGains();
      MS.emit('mixer:changed');
    },

    detach(id) {
      const ch = this.channels.get(id);
      if (!ch) return;
      for (const n of [ch.node, ch.gain, ch.monitor, ...(ch.chain || [])]) {
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
      ch.peak = db > ch.peak ? db : Math.max(db, ch.peak - 2.4);
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
