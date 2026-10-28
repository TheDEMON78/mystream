'use strict';

(function () {
  const MS = window.MS;

  /**
   * Lecteur de GIF animé pour le canevas. Chromium ne fait pas avancer un
   * GIF dessiné depuis une image hors de la page : on décode les images une
   * à une (ImageDecoder) en suivant leurs durées. Seule l'image affichée et
   * la suivante sont en mémoire, même pour un GIF 1080p de plusieurs
   * centaines d'images.
   */
  class GifPlayer {
    constructor(url) {
      this.url = url;
      this.frame = null; // { index, image }
      this.loading = null;
      this.startedAt = 0;
      this.width = 0;
      this.height = 0;
    }

    async load() {
      const res = await fetch(this.url);
      if (!res.ok) throw new Error(`GIF introuvable (${res.status})`);
      this.decoder = new ImageDecoder({ data: res.body, type: 'image/gif' });
      await this.decoder.tracks.ready;
      await this.decoder.completed;
      const track = this.decoder.tracks.selectedTrack;
      this.count = track.frameCount;
      // Les durées ne sont connues qu'en décodant : on les lit une fois, sans garder les images.
      this.durations = [];
      for (let i = 0; i < this.count; i++) {
        const { image } = await this.decoder.decode({ frameIndex: i });
        if (i === 0) {
          this.width = image.displayWidth;
          this.height = image.displayHeight;
        }
        // Comme les navigateurs : une durée nulle ou trop courte vaut 100 ms.
        const ms = image.duration ? image.duration / 1000 : 100;
        this.durations.push(ms < 20 ? 100 : ms);
        if (i === 0) this.frame = { index: 0, image };
        else image.close();
      }
      this.total = this.durations.reduce((a, b) => a + b, 0);
      this.restart();
      return this;
    }

    restart() {
      this.startedAt = performance.now();
    }

    /** Recharge la première image à l'avance (stinger) : la prochaine lecture démarre sans saut. */
    prepare() {
      if (!this.frame || this.frame.index === 0 || this.loading) return;
      this.startedAt = performance.now();
      this.current(false);
    }

    /** Durée d'une lecture complète (ms). */
    get duration() {
      return this.total || 0;
    }

    /** Somme des durées des `n` premières images (ms) : sert au point de transition d'un stinger. */
    timeOfFrame(n) {
      let t = 0;
      for (let i = 0; i < Math.min(n, this.count); i++) t += this.durations[i];
      return t;
    }

    indexAt(elapsed, loop) {
      if (!this.total) return 0;
      if (!loop && elapsed >= this.total) return this.count - 1;
      let t = loop ? elapsed % this.total : elapsed;
      for (let i = 0; i < this.count; i++) {
        t -= this.durations[i];
        if (t < 0) return i;
      }
      return this.count - 1;
    }

    /** Image à afficher maintenant ; l'image suivante se décode en arrière-plan. */
    current(loop = true) {
      if (!this.frame) return null;
      const want = this.indexAt(performance.now() - this.startedAt, loop);
      if (want !== this.frame.index && !this.loading) {
        this.loading = this.decoder
          .decode({ frameIndex: want })
          .then(({ image }) => {
            if (this.closed) return image.close();
            const old = this.frame;
            this.frame = { index: want, image };
            if (old) old.image.close();
          })
          .catch(() => {})
          .finally(() => (this.loading = null));
      }
      return this.frame.image;
    }

    close() {
      this.closed = true;
      if (this.frame) this.frame.image.close();
      this.frame = null;
      if (this.decoder) this.decoder.close();
    }
  }

  MS.GifPlayer = GifPlayer;
  MS.isGif = (file) => /\.gif$/i.test(String(file || ''));
})();
