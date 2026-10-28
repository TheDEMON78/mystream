'use strict';

(function () {
  const MS = window.MS;
  const el = MS.el;
  const PLATFORMS = ['twitch', 'kick'];
  const SPARK_SECONDS = 120;
  const VIEWERS_EVERY = 30000;
  const HISTORY_MAX = 50;

  /** Logo officiel d'une plateforme (mêmes glyphes que le chat). */
  function platformIcon(p, cls = 'pf-icon') {
    return el('span', { class: `${cls} ${p}`, title: p === 'twitch' ? 'Twitch' : 'Kick', html: `<svg viewBox="0 0 24 24"><path d="${MS.PLATFORM_ICONS[p]}"/></svg>` });
  }

  function formatDuration(ms) {
    const m = Math.round(ms / 60000);
    return m >= 60 ? `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, '0')}` : `${m} min`;
  }

  const fmtInt = (n) => Number(n || 0).toLocaleString('fr-FR');
  const fmtPct = (n) => `${n.toLocaleString('fr-FR', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} %`;

  /** Restream est utilisé pour ce live (diffusion ou chat) : on peut lui demander les infos des chaînes. */
  function restreamUsed() {
    const st = MS.store.settings;
    return Boolean(String(st.restream.url || '').trim()) && (st.stream.service === 'restream' || st.chat.mode === 'restream');
  }

  /**
   * Session de live : de « Démarrer le streaming » à l'arrêt définitif
   * (les reconnexions automatiques font partie de la même session).
   */
  class Session {
    constructor() {
      this.startedAt = Date.now();
      this.fps = MS.store.settings.video.fps;
      this.laggedStart = MS.compositor.stats.lagged;
      this.encoderDrops = 0;
      this.lastDrops = new Map(); // compteur FFmpeg par sortie (une par reconnexion)
      this.bitrates = []; // échantillons d'une seconde, pour la courbe
      this.bitrateSum = 0;
      this.bitrateCount = 0;
      this.chat = { twitch: 0, kick: 0 };
      this.chatPerMinute = [];
      this.peak = { twitch: null, kick: null, total: null };
      this.viewers = {};
      this.reconnects = 0;
      this.title = '';
      this.category = '';
    }

    elapsed() {
      return Date.now() - this.startedAt;
    }

    lost() {
      return MS.compositor.stats.lagged - this.laggedStart + this.encoderDrops;
    }

    lostPercent() {
      const expected = (this.elapsed() / 1000) * this.fps;
      return expected > 0 ? Math.min(100, (this.lost() / expected) * 100) : 0;
    }

    sample() {
      const out = MS.outputs.active.stream;
      const p = out && out.progress;
      if (p) {
        const last = this.lastDrops.get(out.id) || 0;
        if (p.dropFrames > last) this.encoderDrops += p.dropFrames - last;
        this.lastDrops.set(out.id, p.dropFrames);
      }
      const rate = p && p.bitrate ? p.bitrate : 0;
      this.bitrates.push(rate);
      if (this.bitrates.length > SPARK_SECONDS) this.bitrates.shift();
      if (rate) {
        this.bitrateSum += rate;
        this.bitrateCount++;
      }
    }

    countMessage(m) {
      if (m.system || m.divider || m.historical || !(m.platform in this.chat)) return;
      this.chat[m.platform]++;
      const i = Math.floor(this.elapsed() / 60000);
      while (this.chatPerMinute.length <= i) this.chatPerMinute.push(0);
      this.chatPerMinute[i]++;
    }

    setViewers(info) {
      let total = null;
      for (const p of PLATFORMS) {
        const v = info[p] && info[p].viewerCount;
        this.viewers[p] = typeof v === 'number' ? v : null;
        if (typeof v === 'number') {
          this.peak[p] = Math.max(this.peak[p] || 0, v);
          total = (total || 0) + v;
        }
      }
      if (total != null) this.peak.total = Math.max(this.peak.total || 0, total);
      const src = (info.twitch && info.twitch.connected && info.twitch) || (info.kick && info.kick.connected && info.kick);
      if (src) {
        this.title = src.title || this.title;
        this.category = src.category || this.category;
      }
    }

    summary() {
      return {
        id: MS.uid(),
        startedAt: this.startedAt,
        duration: this.elapsed(),
        scene: MS.store.currentScene() ? MS.store.currentScene().name : '',
        title: this.title,
        category: this.category,
        chat: { ...this.chat },
        chatPerMinute: this.chatPerMinute.slice(),
        peak: { ...this.peak },
        lost: this.lost(),
        lostPercent: this.lostPercent(),
        reconnects: this.reconnects,
        avgBitrate: this.bitrateCount ? Math.round(this.bitrateSum / this.bitrateCount) : 0,
      };
    }
  }

  const live = {
    session: null,
    wasReconnecting: false,
    info: null,

    init() {
      this.buildBar();
      MS.on('outputs:changed', () => this.sync());
      MS.on('chat:message', (m) => this.session && this.session.countMessage(m));
      MS.on('settings:changed', () => this.render());
      setInterval(() => this.tick(), 1000);
      setInterval(() => this.session && this.pollViewers(), VIEWERS_EVERY);
      this.sync();
    },

    isLive() {
      return Boolean(MS.outputs.active.stream || MS.outputs.reconnect);
    },

    sync() {
      const on = this.isLive();
      if (on && !this.session) {
        this.session = new Session();
        this.wasReconnecting = false;
        this.pollViewers();
      } else if (!on && this.session) {
        const summary = this.session.summary();
        this.session = null;
        this.finish(summary);
      }
      if (this.session) {
        const rc = Boolean(MS.outputs.reconnect);
        if (rc && !this.wasReconnecting) this.session.reconnects++;
        this.wasReconnecting = rc;
      }
      this.render();
    },

    async pollViewers() {
      if (!this.session || !restreamUsed()) return;
      const r = await window.api.restreamLive(MS.store.settings.restream);
      if (!r.ok || !this.session) return;
      this.info = r.data;
      this.session.setViewers(r.data);
      this.render();
    },

    tick() {
      if (!this.session) return;
      this.session.sample();
      this.render();
    },

    /* Barre « En direct » ------------------------------------------------ */
    buildBar() {
      const spark = el('canvas', { class: 'lb-spark', width: 240, height: 28 });
      this.bar = {
        root: el('div', { id: 'livebar', hidden: true }),
        pill: el('span', { class: 'lb-pill', text: 'LIVE' }),
        time: el('span', { class: 'lb-time', text: '00:00:00' }),
        viewers: el('span', { class: 'lb-viewers' }),
        title: el('button', { class: 'lb-title', title: 'Modifier le titre et la catégorie du live', onclick: () => MS.openLiveInfo() }),
        spark,
        rate: el('span', { class: 'lb-rate' }),
        lost: el('button', {
          class: 'lb-lost',
          title: 'Santé de la diffusion',
          onclick: () => MS.toast(MS.outputs.health.message || 'Tout va bien.', 'info', 8000),
        }),
      };
      const b = this.bar;
      b.root.append(b.pill, b.time, b.viewers, b.title, el('span', { class: 'lb-spacer' }), el('span', { class: 'lb-rate-box' }, spark, b.rate), b.lost);
      MS.$('#topbar').after(b.root);
    },

    render() {
      const b = this.bar;
      const s = this.session;
      const show = Boolean(s) && MS.store.settings.ui.liveBar !== false;
      b.root.hidden = !show;
      if (!show) return;
      const rc = MS.outputs.reconnect;
      b.pill.textContent = rc ? 'RECONNEXION' : 'LIVE';
      b.pill.classList.toggle('reconnecting', Boolean(rc));
      b.time.textContent = MS.formatTime(s.elapsed());

      // Spectateurs (via Restream, seulement pour les comptes connectés).
      const parts = [];
      if (restreamUsed() && this.info) {
        for (const p of PLATFORMS) {
          if (!this.info[p] || !this.info[p].connected) continue;
          const v = s.viewers[p];
          parts.push(el('span', { class: 'lb-count', title: `Spectateurs ${p === 'twitch' ? 'Twitch' : 'Kick'}` }, platformIcon(p), el('b', { text: v == null ? '–' : fmtInt(v) })));
        }
      }
      b.viewers.replaceChildren(...parts);
      b.viewers.hidden = !parts.length;

      const title = s.title || (this.info && ((this.info.twitch && this.info.twitch.title) || (this.info.kick && this.info.kick.title)));
      const cat = s.category;
      b.title.hidden = !restreamUsed();
      b.title.textContent = title ? `${title}${cat ? ` · ${cat}` : ''}` : 'Titre et catégorie…';

      const last = s.bitrates[s.bitrates.length - 1] || 0;
      b.rate.textContent = `${fmtInt(Math.round(last))} kb/s`;
      this.drawSpark(s.bitrates);

      const pct = s.lostPercent();
      b.lost.textContent = `${fmtInt(s.lost())} perdues · ${fmtPct(pct)}`;
      b.lost.className = `lb-lost ${pct >= 5 ? 'bad' : pct >= 1 ? 'warn' : ''}`;
    },

    drawSpark(values) {
      const c = this.bar.spark;
      const ctx = c.getContext('2d');
      const W = c.width;
      const H = c.height;
      ctx.clearRect(0, 0, W, H);
      if (values.length < 2) return;
      const target = Math.max(MS.store.settings.stream.videoBitrate + MS.store.settings.stream.audioBitrate, 1);
      const max = Math.max(target * 1.3, ...values);
      const x = (i) => (i / (SPARK_SECONDS - 1)) * W;
      const y = (v) => H - 2 - (v / max) * (H - 4);
      const off = SPARK_SECONDS - values.length;
      // Repère du débit demandé : une chute sous cette ligne se voit tout de suite.
      ctx.strokeStyle = 'rgba(154,160,180,.35)';
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(0, y(target));
      ctx.lineTo(W, y(target));
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.beginPath();
      values.forEach((v, i) => (i ? ctx.lineTo(x(i + off), y(v)) : ctx.moveTo(x(i + off), y(v))));
      ctx.lineTo(x(values.length - 1 + off), H);
      ctx.lineTo(x(off), H);
      ctx.closePath();
      ctx.fillStyle = 'rgba(76,125,255,.18)';
      ctx.fill();
      ctx.beginPath();
      values.forEach((v, i) => (i ? ctx.lineTo(x(i + off), y(v)) : ctx.moveTo(x(i + off), y(v))));
      ctx.strokeStyle = '#4c7dff';
      ctx.lineWidth = 1.5;
      ctx.stroke();
    },

    /* Récap de fin de live --------------------------------------------- */
    finish(summary) {
      // Un live de moins de 30 s est un essai : pas de récap.
      if (summary.duration < 30000) return;
      const data = MS.store.data;
      data.liveHistory = [summary, ...(data.liveHistory || [])].slice(0, HISTORY_MAX);
      MS.store.save();
      this.showRecap(summary);
    },

    showRecap(r) {
      const total = (r.chat.twitch || 0) + (r.chat.kick || 0);
      const tile = (value, label, detail) => el('div', { class: 'recap-tile' }, el('b', { text: value }), el('span', { text: label }), detail ? el('small', { text: detail }) : null);
      const chatDetail = [r.chat.twitch ? `Twitch ${fmtInt(r.chat.twitch)}` : '', r.chat.kick ? `Kick ${fmtInt(r.chat.kick)}` : ''].filter(Boolean).join(' · ');
      const peakDetail = [r.peak.twitch != null ? `Twitch ${fmtInt(r.peak.twitch)}` : '', r.peak.kick != null ? `Kick ${fmtInt(r.peak.kick)}` : ''].filter(Boolean).join(' · ');
      const date = new Date(r.startedAt);
      const content = el(
        'div',
        { class: 'recap' },
        el('p', { class: 'hint', text: `${date.toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' })} à ${date.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })}${r.title ? ` · ${r.title}` : ''}${r.category ? ` (${r.category})` : ''}` }),
        el(
          'div',
          { class: 'recap-tiles' },
          tile(formatDuration(r.duration), 'Durée'),
          tile(fmtInt(total), 'Messages du chat', chatDetail),
          tile(r.peak.total != null ? fmtInt(r.peak.total) : '–', 'Pic de spectateurs', r.peak.total != null ? peakDetail : 'Disponible avec Restream'),
          tile(fmtPct(r.lostPercent), 'Images perdues', `${fmtInt(r.lost)} images`),
          tile(fmtInt(r.reconnects), r.reconnects > 1 ? 'Reconnexions' : 'Reconnexion'),
          tile(r.avgBitrate ? `${fmtInt(r.avgBitrate)}` : '–', 'Débit moyen', 'kb/s')
        ),
        this.chatChart(r)
      );
      MS.modal({ title: 'Fin du live', content, width: 640, buttons: [{ label: 'Historique…', value: 'history', left: true }, { label: 'Fermer', value: true, primary: true }] }).then((v) => {
        if (v === 'history') this.showHistory();
      });
    },

    chatChart(r) {
      const counts = r.chatPerMinute || [];
      const box = el('div', { class: 'recap-chart' });
      if (!counts.some(Boolean)) {
        box.append(el('p', { class: 'hint', text: "Aucun message de chat pendant ce live (chat désactivé ou silencieux)." }));
        return box;
      }
      const max = Math.max(...counts);
      const best = counts.indexOf(max);
      box.append(el('div', { class: 'recap-chart-head' }, el('span', { text: 'Messages par minute' }), el('span', { text: `pic : ${max} messages pendant la minute ${best + 1}` })));
      const W = 600;
      const H = 90;
      const bw = W / counts.length;
      const bars = counts
        .map((c, i) => {
          const h = Math.max(c ? 2 : 0, (c / max) * (H - 6));
          return `<rect x="${(i * bw + bw * 0.12).toFixed(1)}" y="${(H - h).toFixed(1)}" width="${Math.max(1, bw * 0.76).toFixed(1)}" height="${h.toFixed(1)}" rx="1" fill="${i === best ? '#53fc18' : '#4c7dff'}"/>`;
        })
        .join('');
      box.append(el('div', { class: 'recap-svg', html: `<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none">${bars}</svg>` }));
      box.append(el('div', { class: 'recap-axis' }, el('span', { text: '0' }), el('span', { text: formatDuration(r.duration) })));
      return box;
    },

    showHistory() {
      const list = MS.store.data.liveHistory || [];
      const body = list.length
        ? el(
            'table',
            { class: 'history' },
            el('thead', {}, el('tr', {}, ['Date', 'Durée', 'Messages', 'Pic', 'Perdues'].map((t) => el('th', { text: t })))),
            el(
              'tbody',
              {},
              list.map((r) =>
                el(
                  'tr',
                  { title: r.title || '', onclick: () => this.showRecap(r) },
                  el('td', { text: new Date(r.startedAt).toLocaleString('fr-FR', { day: '2-digit', month: '2-digit', year: '2-digit', hour: '2-digit', minute: '2-digit' }) }),
                  el('td', { text: formatDuration(r.duration) }),
                  el('td', { text: fmtInt((r.chat.twitch || 0) + (r.chat.kick || 0)) }),
                  el('td', { text: r.peak.total != null ? fmtInt(r.peak.total) : '–' }),
                  el('td', { text: fmtPct(r.lostPercent) })
                )
              )
            )
          )
        : el('p', { class: 'hint', text: 'Aucun live terminé pour le moment. Le récap de chaque live (plus de 30 s) sera gardé ici.' });
      MS.modal({ title: 'Historique des lives', content: el('div', { class: 'history-box' }, body), width: 620, buttons: [{ label: 'Fermer', value: true, primary: true }] });
    },
  };

  /* Titre et catégorie du live (via Restream) ---------------------------- */
  MS.openLiveInfo = async function () {
    const st = MS.store.settings;
    if (!String(st.restream.url || '').trim()) {
      MS.toast("Le titre et la catégorie passent par Restream, qui a tes comptes Twitch et Kick : renseigne son adresse d'abord.", 'info', 8000);
      MS.openSettings('restream');
      return;
    }
    const r = await window.api.restreamLive(st.restream);
    if (!r.ok) {
      MS.toast(`Restream injoignable : ${r.error}`, 'error', 8000);
      return;
    }
    const info = r.data;
    live.info = info;
    const connected = PLATFORMS.filter((p) => info[p] && info[p].connected);
    const saved = st.liveInfo.titles;
    const current = (connected[0] && info[connected[0]]) || {};

    const title = el('input', { class: 'input', type: 'text', maxlength: 140, value: current.title || '' });
    const category = el('input', { class: 'input', type: 'text', value: current.category || '', placeholder: 'Rechercher (3 lettres minimum)' });
    const suggestions = el('div', { class: 'cat-list' });
    const feedback = el('div', { class: 'live-feedback' });
    const titles = el('div', { class: 'title-chips' });
    let chosen = null; // catégorie choisie dans la liste : { name, twitchId, kickId }
    let searchTimer = null;

    const renderTitles = () => {
      titles.replaceChildren(
        ...saved.map((t, i) =>
          el(
            'span',
            { class: 'title-chip' },
            el('button', { class: 'chip-fill', text: t, title: 'Utiliser ce titre', onclick: () => (title.value = t) }),
            el('button', {
              class: 'chip-del',
              text: '×',
              title: 'Oublier ce titre',
              onclick: () => {
                saved.splice(i, 1);
                MS.store.save();
                renderTitles();
              },
            })
          )
        )
      );
      titles.hidden = !saved.length;
    };
    renderTitles();

    category.addEventListener('input', () => {
      chosen = null;
      clearTimeout(searchTimer);
      const q = category.value.trim();
      if (q.length < 3) {
        suggestions.replaceChildren();
        return;
      }
      searchTimer = setTimeout(async () => {
        const res = await window.api.restreamCategories(st.restream, q);
        if (category.value.trim() !== q) return;
        if (!res.ok) {
          suggestions.replaceChildren(el('div', { class: 'hint', text: res.error }));
          return;
        }
        suggestions.replaceChildren(
          ...(res.data.length
            ? res.data.slice(0, 8).map((c) =>
                el(
                  'button',
                  {
                    class: 'cat-item',
                    onclick: () => {
                      chosen = c;
                      category.value = c.name;
                      suggestions.replaceChildren();
                    },
                  },
                  el('span', { text: c.name }),
                  el('span', { class: 'cat-pf' }, c.twitchId ? platformIcon('twitch') : null, c.kickId ? platformIcon('kick') : null)
                )
              )
            : [el('div', { class: 'hint', text: connected.length > 1 ? 'Aucune catégorie portant exactement ce nom sur Twitch et sur Kick.' : 'Aucune catégorie trouvée.' })])
        );
      }, 350);
    });

    const status = el(
      'div',
      { class: 'live-platforms' },
      PLATFORMS.map((p) => {
        const i = info[p] || {};
        const label = i.connected ? `connecté${typeof i.viewerCount === 'number' ? ` · ${fmtInt(i.viewerCount)} spectateurs` : ''}` : i.error ? 'injoignable' : 'non connecté dans Restream';
        return el('span', { class: `live-pf ${i.connected ? 'on' : ''}` }, platformIcon(p), el('span', { text: `${p === 'twitch' ? 'Twitch' : 'Kick'} : ${label}` }));
      })
    );

    const content = el(
      'div',
      { class: 'live-info' },
      status,
      el('label', { class: 'field' }, el('span', { text: 'Titre' }), title),
      el(
        'div',
        { class: 'row-actions' },
        el('button', {
          class: 'btn small',
          text: '☆ Enregistrer ce titre',
          onclick: () => {
            const t = title.value.trim();
            if (!t || saved.includes(t)) return;
            saved.unshift(t);
            saved.splice(20);
            MS.store.save();
            renderTitles();
          },
        })
      ),
      titles,
      el('label', { class: 'field' }, el('span', { text: 'Catégorie' }), category),
      suggestions,
      connected.length ? null : el('p', { class: 'hint', text: "Aucun compte connecté : connecte Twitch et/ou Kick dans l'interface de Restream." }),
      feedback
    );

    let closeModal = null;
    const apply = async () => {
      const t = title.value.trim();
      const changedCat = category.value.trim() && category.value.trim() !== (current.category || '');
      if (changedCat && !chosen) {
        feedback.className = 'live-feedback error';
        feedback.textContent = 'Choisis la catégorie dans la liste proposée.';
        return;
      }
      if (!t && !chosen) return;
      feedback.className = 'live-feedback';
      feedback.textContent = 'Mise à jour…';
      const res = await window.api.restreamUpdateInfo(st.restream, {
        title: t,
        twitchCategoryId: chosen ? chosen.twitchId || undefined : undefined,
        twitchCategoryName: chosen && chosen.twitchId ? chosen.name : undefined,
        kickCategoryId: chosen ? chosen.kickId || undefined : undefined,
        kickCategoryName: chosen && chosen.kickId ? chosen.name : undefined,
      });
      if (!res.ok) {
        feedback.className = 'live-feedback error';
        feedback.textContent = res.error;
        return;
      }
      const results = Object.entries(res.data);
      const failed = results.filter(([, v]) => !v.ok);
      if (!results.length) {
        feedback.className = 'live-feedback error';
        feedback.textContent = 'Aucune plateforme connectée dans Restream.';
        return;
      }
      if (failed.length) {
        feedback.className = 'live-feedback error';
        feedback.textContent = failed.map(([p, v]) => `${p} : ${v.error}`).join(' · ');
        return;
      }
      if (live.session) {
        live.session.title = t || live.session.title;
        if (chosen) live.session.category = chosen.name;
      }
      MS.toast(`Titre${chosen ? ' et catégorie' : ''} mis à jour sur ${results.map(([p]) => (p === 'twitch' ? 'Twitch' : 'Kick')).join(' et ')}.`, 'success', 4000);
      live.pollViewers();
      closeModal(true);
    };

    MS.modal({
      title: 'Informations du live',
      content,
      width: 520,
      onOpen: (close) => (closeModal = close),
      buttons: [
        { label: 'Annuler', value: false },
        { label: 'Mettre à jour', primary: true, onClick: apply },
      ],
    });
  };

  MS.live = live;
})();
