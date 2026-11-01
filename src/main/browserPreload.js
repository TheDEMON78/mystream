'use strict';

// Préchargement des sources « Navigateur » : capture le son de la page
// (éléments <audio>/<video> des alertes, Web Audio des bibliothèques comme
// Howler) pour l'envoyer au mixeur de MyStream, comme « Contrôler l'audio
// via OBS ». Le son ne sort plus par les haut-parleurs du PC : on l'écoute
// avec le bouton casque du mixeur.
const { contextBridge, ipcRenderer, webFrame } = require('electron');

if (ipcRenderer.sendSync('browser:audio-enabled')) {
  contextBridge.exposeInMainWorld('__myStreamAudio', { push: (pcm) => ipcRenderer.send('browser:audio', pcm) });
  // Le crochet doit s'installer dans le monde de la page, avant ses propres scripts.
  webFrame.executeJavaScript(`(() => {
    const push = window.__myStreamAudio.push;
    delete window.__myStreamAudio;
    const Native = window.AudioContext;
    const realDestination = Object.getOwnPropertyDescriptor(BaseAudioContext.prototype, 'destination').get;
    let capture = null;
    let mix = null;

    // Contexte de capture unique à 48 kHz : tout le son de la page y est mélangé puis envoyé à MyStream.
    function sink() {
      if (capture) return mix;
      capture = new Native({ sampleRate: 48000 });
      mix = capture.createGain();
      const tap = capture.createScriptProcessor(1024, 2, 2);
      const silent = capture.createGain();
      silent.gain.value = 0;
      mix.connect(tap);
      tap.connect(silent);
      silent.connect(realDestination.call(capture));
      tap.onaudioprocess = (e) => {
        const l = e.inputBuffer.getChannelData(0);
        const r = e.inputBuffer.getChannelData(1);
        let sound = false;
        for (let i = 0; i < l.length; i += 16) if (l[i] || r[i]) { sound = true; break; }
        if (!sound) return; // pas d'envoi pendant les silences
        const out = new Float32Array(l.length * 2);
        for (let i = 0; i < l.length; i++) { out[2 * i] = l[i]; out[2 * i + 1] = r[i]; }
        push(out);
      };
      return mix;
    }

    // Web Audio : la « sortie » de chaque contexte de la page est détournée vers la capture.
    class CapturedAudioContext extends Native {
      constructor(...args) {
        super(...args);
        const real = realDestination.call(this);
        const out = this.createGain();
        const stream = this.createMediaStreamDestination();
        out.connect(stream);
        sink().context.createMediaStreamSource(stream.stream).connect(mix);
        Object.defineProperty(out, 'maxChannelCount', { value: real.maxChannelCount });
        Object.defineProperty(this, 'destination', { get: () => out });
      }
    }
    window.AudioContext = CapturedAudioContext;
    if (window.webkitAudioContext) window.webkitAudioContext = CapturedAudioContext;

    // Éléments <audio> et <video> : reliés à la capture par MyStream une fois la page
    // chargée (Chromium retire les écouteurs posés avant le chargement du document).
    const seen = new WeakSet();
    Object.defineProperty(window, '__myStreamCaptureMedia', {
      value: (el) => {
        if (!(el instanceof HTMLMediaElement) || seen.has(el)) return;
        seen.add(el);
        try { sink().context.createMediaElementSource(el).connect(mix); } catch (err) { /* déjà relié */ }
      },
    });
  })();`);
}
