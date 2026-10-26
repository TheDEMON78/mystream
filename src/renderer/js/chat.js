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

  /* Twitch : lecture anonyme du chat IRC (aucun compte nécessaire) ------- */
  class TwitchIrc {
    constructor(channel, onMessage, onStatus) {
      this.channel = channel.toLowerCase().replace(/^.*twitch\.tv\//, '').replace(/[^a-z0-9_]/g, '');
      this.onMessage = onMessage;
      this.onStatus = onStatus;
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
            return { type, count: type === 'subscriber' || type === 'bits' ? Number(v) || undefined : undefined };
          });
        const name = (tags['display-name'] || '').replace(/\\s/g, ' ') || m[1];
        this.onMessage({ platform: 'twitch', username: name, color: tags.color || null, message: text, badges });
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

  /* Kick : lecture du chat public via Pusher ------------------------------ */
  class KickPusher {
    constructor(cfg, onMessage, onStatus) {
      this.cfg = cfg;
      this.onMessage = onMessage;
      this.onStatus = onStatus;
      this.stopped = false;
      this.start();
    }

    async start() {
      let id = String(this.cfg.kickChatroomId || '').trim();
      if (!id) {
        this.onStatus('connecting');
        try {
          id = await window.api.kickChatroom(this.cfg.kickChannel);
        } catch (e) {
          this.onStatus('error', String(e.message || e).replace(/^Error invoking remote method '[^']+': (Error: )?/, ''));
          return;
        }
      }
      if (this.stopped) return;
      this.chatroom = id;
      this.connect();
    }

    connect() {
      const ws = new WebSocket('wss://ws-us2.pusher.com/app/32cbd69e4b950bf97679?protocol=7&client=js&version=8.4.0&flash=false');
      this.ws = ws;
      this.onStatus('connecting');
      ws.onmessage = (e) => {
        let msg;
        try {
          msg = JSON.parse(e.data);
        } catch {
          return;
        }
        if (msg.event === 'pusher:connection_established') {
          ws.send(JSON.stringify({ event: 'pusher:subscribe', data: { auth: '', channel: `chatrooms.${this.chatroom}.v2` } }));
        } else if (msg.event === 'pusher_internal:subscription_succeeded') {
          this.onStatus('connected');
        } else if (msg.event === 'pusher:ping') {
          ws.send(JSON.stringify({ event: 'pusher:pong', data: {} }));
        } else if (msg.event === 'App\\Events\\ChatMessageEvent') {
          const d = typeof msg.data === 'string' ? JSON.parse(msg.data) : msg.data;
          const sender = d.sender || {};
          const identity = sender.identity || {};
          this.onMessage({
            platform: 'kick',
            username: sender.username || 'inconnu',
            color: identity.color || null,
            message: String(d.content || '').replace(/\[emote:\d+:([^\]]+)\]/g, '$1'),
            badges: (identity.badges || []).map((b) => ({ type: (b.type || '').replace(/-/g, '_'), count: b.count })),
          });
        }
      };
      ws.onclose = () => {
        if (this.stopped) return;
        this.onStatus('error', 'déconnecté, nouvelle tentative…');
        this.timer = setTimeout(() => this.connect(), 3000);
      };
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
      if (c.mode === 'direct') {
        if (c.twitchChannel.trim()) {
          this.providers.push(new TwitchIrc(c.twitchChannel, (m) => this.push(m), (s, e) => this.setStatus('twitch', s, e)));
        }
        if (c.kickChannel.trim() || String(c.kickChatroomId).trim()) {
          this.providers.push(new KickPusher(c, (m) => this.push(m), (s, e) => this.setStatus('kick', s, e)));
        }
      }
      MS.emit('chat:status');
    },

    setStatus(platform, status, error) {
      this.status[platform] = { status, error: error || null };
      MS.emit('chat:status');
    },

    push(m) {
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
      this.messages.push(m);
      if (this.messages.length > MAX_MESSAGES) this.messages.shift();
      MS.emit('chat:message', m);
    },

    clear() {
      this.messages = [];
      MS.emit('chat:cleared');
    },
  };

  MS.chat = chat;
})();
