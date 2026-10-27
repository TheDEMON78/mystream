'use strict';

(function () {
  const MS = window.MS;
  const MAX_MESSAGES = 200;

  // Glyphes officiels (simple-icons, CC0), identiques à ceux de Restream.
  MS.PLATFORM_ICONS = {
    twitch: 'M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z',
    kick: 'M1.333 0h8v5.333H12V2.667h2.667V0h8v8H20v2.667h-2.667v2.666H20V16h2.667v8h-8v-2.667H12v-2.666H9.333V24h-8Z',
  };
  MS.PLATFORM_COLORS = { twitch: '#a970ff', kick: '#53fc18', restream: '#4c7dff' };

  MS.BADGE_LABELS = {
    broadcaster: 'Diffuseur',
    moderator: 'Modo',
    vip: 'VIP',
    subscriber: 'Abonné',
    founder: 'Fondateur',
    staff: 'Staff',
    admin: 'Admin',
    partner: 'Partenaire',
    turbo: 'Turbo',
    premium: 'Prime',
    bits: 'Bits',
    sub_gifter: 'Cadeau abo',
    og: 'OG',
  };

  // Logos intégrés des badges (Kick ne fournit aucune image de badge) :
  // fond coloré + pictogramme blanc dessiné au trait, sur une grille 24×24.
  MS.BADGE_ICONS = {
    broadcaster: { bg: '#e91916', d: 'M4 7h10v10H4zM14 10.5l5-3v9l-5-3z' },
    moderator: { bg: '#00ad03', d: 'M15 4h5v5L10 19l-5-5zM4 20l3-3' },
    vip: { bg: '#e005b9', d: 'M7 5h10l4 5-9 10L3 10zM3 10h18' },
    subscriber: { bg: '#8205b4', d: 'M12 4l2.4 5 5.6.6-4.2 3.8 1.2 5.6-5-2.9-5 2.9 1.2-5.6L4 9.6 9.6 9z' },
    founder: { bg: '#3d89d8', d: 'M4 17h16M5 17l-1-9 5 4 3-6 3 6 5-4-1 9' },
    og: { bg: '#0e9e7a', d: 'M8.5 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM20 9h-3a2 2 0 0 0-2 2v2a2 2 0 0 0 2 2h3v-3h-2' },
    verified: { bg: '#1fd655', d: 'M5 12.5l4.5 4.5L19 7.5' },
    sub_gifter: { bg: '#ff8c00', d: 'M4 10h16v3H4zM6 13h12v7H6zM12 10v10M12 10c-2-5-6-4-4-.5M12 10c2-5 6-4 4-.5' },
    staff: { bg: '#3a3a3a', d: 'M14 6a4 4 0 0 0 5 5l-8 8-3-3 8-8a4 4 0 0 1-2-2z' },
    partner: { bg: '#9146ff', d: 'M5 12.5l4.5 4.5L19 7.5' },
    bits: { bg: '#8a3dff', d: 'M12 3l7 9-7 9-7-9z' },
    premium: { bg: '#00a3ff', d: 'M4 8l4 4 4-7 4 7 4-4-2 10H6z' },
  };

  /** Découpe le texte d'un message en morceaux de texte et d'emotes. */
  function parseParts(m) {
    const text = String(m.message || '');
    const parts = [];
    if (m.emotes && m.emotes.length) {
      // Positions Twitch exprimées en caractères Unicode (pas en UTF-16).
      const chars = Array.from(text);
      let pos = 0;
      for (const e of [...m.emotes].sort((a, b) => a.start - b.start)) {
        if (e.start > pos) parts.push({ type: 'text', text: chars.slice(pos, e.start).join('') });
        parts.push({ type: 'emote', name: chars.slice(e.start, e.end + 1).join(''), url: `https://static-cdn.jtvnw.net/emoticons/v2/${e.id}/default/dark/2.0` });
        pos = e.end + 1;
      }
      if (pos < chars.length) parts.push({ type: 'text', text: chars.slice(pos).join('') });
      return parts;
    }
    // Emotes Kick : [emote:ID:nom]
    const re = /\[emote:(\d+):([^\]]*)\]/g;
    let last = 0;
    let match;
    while ((match = re.exec(text))) {
      if (match.index > last) parts.push({ type: 'text', text: text.slice(last, match.index) });
      parts.push({ type: 'emote', name: match[2], url: `https://files.kick.com/emotes/${match[1]}/fullsize` });
      last = re.lastIndex;
    }
    if (last < text.length) parts.push({ type: 'text', text: text.slice(last) });
    return parts;
  }

  /** Cache d'images (badges, emotes) chargées en CORS pour pouvoir les dessiner dans le flux. */
  const images = new Map();
  MS.chatImage = function (url) {
    let img = images.get(url);
    if (!img) {
      img = new Image();
      img.crossOrigin = 'anonymous';
      img.onload = () => MS.emit('chat:image');
      img.onerror = () => (img.failed = true);
      img.src = url;
      images.set(url, img);
      if (images.size > 500) images.delete(images.keys().next().value);
    }
    return img.complete && img.naturalWidth && !img.failed ? img : null;
  };

  /* Twitch : lecture anonyme du chat IRC (aucun compte nécessaire) ------- */
  class TwitchIrc {
    constructor(channel, onMessage, onStatus, onModeration) {
      this.channel = channel.toLowerCase().replace(/^.*twitch\.tv\//, '').replace(/[^a-z0-9_]/g, '');
      this.onMessage = onMessage;
      this.onStatus = onStatus;
      this.onModeration = onModeration;
      this.stopped = false;
      this.connect();
    }

    connect() {
      const ws = new WebSocket('wss://irc-ws.chat.twitch.tv:443');
      this.ws = ws;
      this.onStatus('connecting');
      ws.onopen = () => {
        ws.send('CAP REQ :twitch.tv/tags twitch.tv/commands');
        ws.send('PASS SCHMOOPIIE');
        ws.send(`NICK justinfan${Math.floor(10000 + Math.random() * 80000)}`);
        ws.send(`JOIN #${this.channel}`);
      };
      ws.onmessage = (e) => {
        for (const line of String(e.data).split('\r\n')) if (line) this.handle(line);
      };
      ws.onclose = () => {
        if (this.stopped) return;
        this.onStatus('error', 'déconnecté, nouvelle tentative…');
        this.timer = setTimeout(() => this.connect(), 3000);
      };
    }

    handle(line) {
      if (line.startsWith('PING')) {
        this.ws.send('PONG :tmi.twitch.tv');
        return;
      }
      const tags = {};
      let rest = line;
      if (rest[0] === '@') {
        const i = rest.indexOf(' ');
        for (const kv of rest.slice(1, i).split(';')) {
          const j = kv.indexOf('=');
          tags[kv.slice(0, j)] = kv.slice(j + 1);
        }
        rest = rest.slice(i + 1);
      }
      const m = rest.match(/^:([^!]+)![^ ]+ PRIVMSG #[^ ]+ :(.*)$/);
      if (m) {
        let text = m[2];
        const action = text.match(/^\u0001ACTION (.*)\u0001$/);
        if (action) text = action[1];
        const badges = (tags.badges || '')
          .split(',')
          .filter(Boolean)
          .map((b) => {
            const [type, v] = b.split('/');
            return { type, version: v, count: type === 'subscriber' || type === 'bits' ? Number(v) || undefined : undefined };
          });
        const emotes = [];
        for (const group of (tags.emotes || '').split('/').filter(Boolean)) {
          const [id, ranges] = group.split(':');
          for (const r of (ranges || '').split(',')) {
            const [start, end] = r.split('-').map(Number);
            if (Number.isFinite(start) && Number.isFinite(end)) emotes.push({ id, start, end });
          }
        }
        // Un /me (ACTION) décale le texte : les positions d'emotes ne s'appliquent plus.
        const name = (tags['display-name'] || '').replace(/\\s/g, ' ') || m[1];
        this.onMessage({ id: tags.id, userId: tags['user-id'], platform: 'twitch', username: name, color: tags.color || null, message: text, badges, emotes: action ? [] : emotes });
      } else if (/^:tmi\.twitch\.tv CLEARMSG #/.test(rest)) {
        // Message supprimé par un modérateur.
        this.onModeration({ platform: 'twitch', action: 'delete', id: tags['target-msg-id'] });
      } else if (/^:tmi\.twitch\.tv CLEARCHAT #/.test(rest)) {
        // Viewer banni ou en timeout (ses messages disparaissent), sinon /clear.
        const user = rest.match(/ CLEARCHAT #[^ ]+ :(.+)$/);
        this.onModeration(user ? { platform: 'twitch', action: 'user', userId: tags['target-user-id'], username: user[1] } : { platform: 'twitch', action: 'clear' });
      } else if (/ ROOMSTATE #/.test(rest) || / JOIN #/.test(rest)) {
        this.onStatus('connected');
      }
    }

    stop() {
      this.stopped = true;
      clearTimeout(this.timer);
      if (this.ws) this.ws.close();
    }
  }

  /* Gestionnaire --------------------------------------------------------- */
  const chat = {
    messages: [],
    status: {},
    lastTs: 0,
    replayed: false,
    providers: [],
    sig: null,

    init() {
      window.api.onChatMessage((m) => this.push(m));
      window.api.onChatStatus((s) => this.setStatus(s.platform, s.status, s.error));
      window.api.onChatModeration((a) => this.moderate(a));
      MS.on('settings:changed', () => this.apply());
      this.apply();
    },

    apply() {
      const st = MS.store.settings;
      const c = st.chat;
      const sig = JSON.stringify([c, c.mode === 'restream' ? st.restream : null]);
      if (sig === this.sig) return;
      this.sig = sig;

      this.providers.forEach((p) => p.stop());
      this.providers = [];
      this.status = {};
      window.api.restreamChat(c.mode === 'restream', st.restream);
      // Kick en direct : page de chat officielle « popout » lue en arrière-plan.
      window.api.kickChat(c.mode === 'direct' ? c.kickChannel : null);
      this.badgeMap = null;
      if (c.mode === 'direct' && c.twitchChannel.trim()) {
        this.providers.push(new TwitchIrc(c.twitchChannel, (m) => this.push(m), (s, e) => this.setStatus('twitch', s, e), (a) => this.moderate(a)));
        window.api.twitchBadges(c.twitchChannel).then((map) => {
          this.badgeMap = map;
          this.messages.forEach((m) => this.resolveBadges(m));
          MS.emit('chat:cleared'); // ré-affiche les messages déjà reçus avec leurs badges
        });
      }
      MS.emit('chat:status');
    },

    setStatus(platform, status, error) {
      this.status[platform] = { status, error: error || null };
      MS.emit('chat:status');
    },

    push(m) {
      // Restream peut relayer les actions de modération dans le même flux.
      if (m.moderation) return this.moderate({ platform: m.platform, ...m.moderation });
      // Restream rejoue un court historique à chaque (re)connexion : on
      // ignore ce qui a déjà été affiché pour éviter les doublons.
      if (m.historical) {
        if (m.timestamp && m.timestamp <= this.lastTs) return;
        this.replayed = true;
      }
      if (m.divider) {
        if (!this.replayed) return;
        this.replayed = false;
      }
      if (!m.system && !m.divider && m.timestamp) this.lastTs = Math.max(this.lastTs, m.timestamp);
      m.at = m.historical ? 0 : Date.now();
      if (!m.system && !m.divider) m.parts = parseParts(m);
      this.resolveBadges(m);
      this.messages.push(m);
      if (this.messages.length > MAX_MESSAGES) this.messages.shift();
      MS.emit('chat:message', m);
    },

    /**
     * Retire du chat (dock et incrustation) ce que la plateforme a supprimé :
     * un message (`delete`), tous ceux d'un viewer banni ou en timeout
     * (`user`), ou tout le chat de la plateforme (`clear`).
     */
    moderate(a) {
      if (!a || !a.platform) return;
      const name = String(a.username || '').toLowerCase();
      const hit = (m) => {
        if (m.platform !== a.platform || m.system || m.divider) return false;
        if (a.action === 'clear') return true;
        if (a.action === 'delete') return !!a.id && m.id === a.id;
        if (a.action === 'user') return (a.userId != null && m.userId != null && String(m.userId) === String(a.userId)) || (!!name && String(m.username || '').toLowerCase() === name);
        return false;
      };
      const kept = this.messages.filter((m) => !hit(m));
      if (kept.length === this.messages.length) return;
      this.messages = kept;
      MS.emit('chat:cleared');
    },

    /** Images des badges Twitch en lecture directe (Restream fournit déjà les siennes). */
    resolveBadges(m) {
      if (m.platform !== 'twitch' || !this.badgeMap) return;
      for (const b of m.badges || []) {
        if (b.imageUrl) continue;
        const found = this.badgeMap[`${b.type}/${b.version}`] || this.badgeMap[`${b.type}/1`] || this.badgeMap[`${b.type}/0`];
        if (found) Object.assign(b, { imageUrl: found.url, title: found.title });
      }
    },

    clear() {
      this.messages = [];
      MS.emit('chat:cleared');
    },
  };

  MS.chat = chat;
})();
