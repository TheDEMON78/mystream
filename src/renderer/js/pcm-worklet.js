'use strict';

// Lecteur PCM pour le son capturé par window-audio.exe : tampon circulaire
// avec latence maîtrisée (pas de dérive si l'interface prend du retard).
const CAPACITY = 48000; // 1 s de son stéréo
const START_AT = 960; // 20 ms de réserve avant de jouer
const MAX_LATENCY = 9600; // au-delà de 200 ms en attente, on rattrape
const TARGET = 2400; // ... pour revenir à 50 ms

class PcmPlayer extends AudioWorkletProcessor {
  constructor() {
    super();
    this.left = new Float32Array(CAPACITY);
    this.right = new Float32Array(CAPACITY);
    this.read = 0;
    this.write = 0;
    this.count = 0;
    this.playing = false;
    this.port.onmessage = (e) => this.push(e.data);
  }

  push(data) {
    const frames = data.length >> 1;
    for (let i = 0; i < frames; i++) {
      this.left[this.write] = data[2 * i];
      this.right[this.write] = data[2 * i + 1];
      this.write = (this.write + 1) % CAPACITY;
      if (this.count < CAPACITY) this.count++;
      else this.read = (this.read + 1) % CAPACITY;
    }
    if (this.count > MAX_LATENCY) {
      const skip = this.count - TARGET;
      this.read = (this.read + skip) % CAPACITY;
      this.count -= skip;
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const l = out[0];
    const r = out[1] || out[0];
    if (!this.playing && this.count >= START_AT) this.playing = true;
    for (let i = 0; i < l.length; i++) {
      if (this.playing && this.count > 0) {
        l[i] = this.left[this.read];
        r[i] = this.right[this.read];
        this.read = (this.read + 1) % CAPACITY;
        this.count--;
      } else {
        l[i] = 0;
        r[i] = 0;
        this.playing = false;
      }
    }
    return true;
  }
}

registerProcessor('pcm-player', PcmPlayer);
