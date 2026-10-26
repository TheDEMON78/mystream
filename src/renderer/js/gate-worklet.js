'use strict';

// Noise gate : coupe le micro sous un seuil (bruit de fond, clavier,
// ventilateur) et le rouvre dès qu'on parle. Tourne dans le thread audio.
class NoiseGate extends AudioWorkletProcessor {
  static get parameterDescriptors() {
    return [
      { name: 'threshold', defaultValue: -45, minValue: -100, maxValue: 0 },
      { name: 'release', defaultValue: 150, minValue: 10, maxValue: 2000 },
    ];
  }

  constructor() {
    super();
    this.env = 0;
    this.gain = 0;
    this.hold = 0;
  }

  process(inputs, outputs, params) {
    const input = inputs[0];
    const output = outputs[0];
    if (!input || !input.length) return true;
    const thr = Math.pow(10, params.threshold[0] / 20);
    const attack = Math.exp(-1 / (sampleRate * 0.002));
    const release = Math.exp(-1 / ((sampleRate * params.release[0]) / 1000));
    const decay = Math.exp(-1 / (sampleRate * 0.04));
    const holdSamples = sampleRate * 0.2;
    const n = input[0].length;
    for (let i = 0; i < n; i++) {
      let peak = 0;
      for (let c = 0; c < input.length; c++) {
        const v = Math.abs(input[c][i]);
        if (v > peak) peak = v;
      }
      this.env = peak > this.env ? peak : this.env * decay;
      if (this.env > thr) this.hold = holdSamples;
      else if (this.hold > 0) this.hold--;
      const target = this.env > thr || this.hold > 0 ? 1 : 0;
      this.gain = target + (this.gain - target) * (target > this.gain ? attack : release);
      for (let c = 0; c < output.length; c++) output[c][i] = (input[c] || input[0])[i] * this.gain;
    }
    return true;
  }
}

registerProcessor('noise-gate', NoiseGate);
