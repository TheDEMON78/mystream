'use strict';

(function () {
  const MS = window.MS;
  const el = MS.el;

  /*
   * Import d'une collection de scènes OBS Studio (Collection de scènes >
   * Exporter, ou %APPDATA%\obs-studio\basic\scenes\*.json).
   *
   * Chaque source OBS est convertie vers son équivalent MyStream ; ce qui n'a
   * pas d'équivalent exact est adapté et signalé dans le rapport. Les
   * réglages propres à MyStream (stream, chat, Restream, docks, raccourcis…)
   * ne sont jamais touchés.
   */

  const OBS_DEFAULT_CSS = 'body { background-color: rgba(0, 0, 0, 0); margin: 0px auto; overflow: hidden; }';
  const CHAT_WIDGETS = /streamlabs\.com\/widgets\/chat-box|streamelements\.com\/.*chat|twitch\.tv\/.*\/chat|kick\.com\/.*chat|\/chat\.html|chatis|kapchat/i;

  MS.isObsCollection = (data) => Boolean(data && Array.isArray(data.sources) && Array.isArray(data.scene_order));

  /** Couleur OBS (entier 0xAABBGGRR) → { hex, alpha }. */
  function obsColor(v, fallback = 0xffffffff) {
    const c = (v == null ? fallback : Number(v)) >>> 0;
    const hex = (n) => n.toString(16).padStart(2, '0');
    return { hex: `#${hex(c & 255)}${hex((c >>> 8) & 255)}${hex((c >>> 16) & 255)}`, alpha: ((c >>> 24) & 255) / 255 };
  }

  /** « Titre:Classe:programme.exe » d'OBS (« : » encodé #3A, « # » encodé #22). */
  function parseWindow(str) {
    const dec = (t) => String(t || '').replace(/#3A/g, ':').replace(/#22/g, '#');
    const [title, cls, exe] = String(str || '').split(':');
    return { title: dec(title).trim(), cls: dec(cls), exe: dec(exe) };
  }

  const baseName = (p) => String(p || '').split(/[\\/]/).pop();
  const ext = (p) => (String(p || '').match(/\.([a-z0-9]+)$/i) || [, ''])[1].toLowerCase();
  const IMAGE_EXT = ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'svg', 'gif', 'tga'];

  /** Place un élément OBS (alignement, cadre) une fois la taille de sa source connue. */
  MS.obsPlacement = function (p, w, h) {
    const fx = (a) => (a & 1 ? 0 : a & 2 ? 1 : 0.5);
    const fy = (a) => (a & 4 ? 0 : a & 8 ? 1 : 0.5);
    let sx = p.scale.x;
    let sy = p.scale.y;
    let boxW = w * sx;
    let boxH = h * sy;
    if (p.boundsType) {
      const bw = p.bounds.x;
      const bh = p.bounds.y;
      const fit = Math.min(bw / w, bh / h);
      const cover = Math.max(bw / w, bh / h);
      const scales = { 1: [bw / w, bh / h], 2: [fit, fit], 3: [cover, cover], 4: [bw / w, bw / w], 5: [bh / h, bh / h], 6: [Math.min(1, fit), Math.min(1, fit)] };
      [sx, sy] = scales[p.boundsType] || [sx, sy];
      boxW = bw;
      boxH = bh;
    }
    const bx = p.pos.x - boxW * fx(p.align);
    const by = p.pos.y - boxH * fy(p.align);
    const x = bx + (boxW - w * sx) * fx(p.boundsType ? p.boundsAlign : 5);
    const y = by + (boxH - h * sy) * fy(p.boundsType ? p.boundsAlign : 5);
    const f = p.f || 1;
    return { x: Math.round(x * f), y: Math.round(y * f), scaleX: sx * f, scaleY: sy * f };
  };

  /* Filtres ------------------------------------------------------------------ */

  function convertFilters(obs, src, notes) {
    const audio = { ...MS.audioFilterDefaults() };
    const video = { ...MS.filterDefaults() };
    let hasAudio = false;
    let hasVideo = false;
    for (const f of obs.filters || []) {
      if (f.enabled === false) continue;
      const s = f.settings || {};
      switch (f.id) {
        case 'gain_filter':
          audio.gain = MS.clamp(Number(s.db) || 0, -30, 30);
          hasAudio = true;
          break;
        case 'noise_gate_filter':
          audio.gate = true;
          audio.gateThreshold = Number(s.open_threshold ?? -26);
          audio.gateRelease = Number(s.release_time ?? 150);
          hasAudio = true;
          break;
        case 'compressor_filter':
          audio.compressor = true;
          audio.compThreshold = Number(s.threshold ?? -18);
          audio.compRatio = Number(s.ratio ?? 10);
          audio.compMakeup = Number(s.output_gain ?? 0);
          hasAudio = true;
          break;
        case 'limiter_filter':
          audio.limiter = true;
          hasAudio = true;
          break;
        case 'noise_suppress_filter':
        case 'noise_suppress_filter_v2':
          if (src.type === 'audio_input') src.settings.noiseSuppression = true;
          else notes.push({ level: 'todo', text: `« ${obs.name} » : la suppression du bruit n'existe que pour les micros dans MyStream, filtre ignoré.` });
          break;
        case 'crop_filter':
          if (s.relative === false) {
            notes.push({ level: 'todo', text: `« ${obs.name} » : recadrage en taille absolue non repris, à refaire dans Filtres.` });
            break;
          }
          Object.assign(video, { cropLeft: Number(s.left) || 0, cropTop: Number(s.top) || 0, cropRight: Number(s.right) || 0, cropBottom: Number(s.bottom) || 0 });
          hasVideo = true;
          break;
        case 'color_filter':
        case 'color_filter_v2': {
          const v2 = f.id === 'color_filter_v2';
          const op = s.opacity == null ? 1 : v2 ? Number(s.opacity) : Number(s.opacity) / 100;
          video.opacity = Math.round(MS.clamp(op, 0, 1) * 100);
          video.brightness = Math.round(MS.clamp(100 + (Number(s.brightness) || 0) * 100, 0, 200));
          video.contrast = Math.round(MS.clamp(100 + (Number(s.contrast) || 0) * 50, 0, 300));
          video.saturation = Math.round(MS.clamp(100 + (Number(s.saturation) || 0) * 100, 0, 300));
          video.hue = Math.round(Number(s.hue_shift) || 0);
          hasVideo = true;
          break;
        }
        case 'chroma_key_filter':
        case 'chroma_key_filter_v2':
        case 'color_key_filter':
        case 'color_key_filter_v2': {
          const presets = { green: '#00ff00', blue: '#0000ff', magenta: '#ff00ff', red: '#ff0000' };
          video.chroma = true;
          video.chromaColor = presets[s.key_color_type || 'green'] || obsColor(s.key_color, 0xff00ff00).hex;
          video.similarity = Number(s.similarity ?? 400);
          video.smoothness = Number(s.smoothness ?? 80);
          hasVideo = true;
          if (f.id.startsWith('color_key')) notes.push({ level: 'info', text: `« ${obs.name} » : clé de couleur convertie en incrustation (fond vert), à vérifier.` });
          break;
        }
        default:
          notes.push({ level: 'todo', text: `« ${obs.name} » : filtre « ${f.name} » (${f.id}) sans équivalent dans MyStream, ignoré.` });
      }
    }
    if (hasAudio) src.audioFilters = audio;
    if (hasVideo) src.filters = video;
  }

  /* Sources ----------------------------------------------------------------- */

  /** Convertit une source OBS ; renvoie null si elle n'a pas d'équivalent. */
  function convertSource(obs, ctx) {
    const s = obs.settings || {};
    const notes = ctx.notes;
    const mk = (type, settings, extra = {}) => ({
      id: MS.uid(),
      type,
      name: obs.name,
      settings: { ...MS.SourceTypes[type].defaults(), ...settings },
      volume: obs.volume == null ? 1 : Number(obs.volume),
      muted: Boolean(obs.muted),
      monitor: Number(obs.monitoring_type) > 0,
      ...extra,
    });
    switch (obs.versioned_id || obs.id) {
      case 'browser_source': {
        const url = s.is_local_file ? s.local_file : s.url;
        if (ctx.options.replaceChat && CHAT_WIDGETS.test(String(url || ''))) {
          notes.push({ level: 'info', text: `« ${obs.name} » (widget de chat) remplacé par la source « Chat du stream » de MyStream (Twitch + Kick, réglages dans Paramètres > Chat).` });
          return mk('chat', { width: Number(s.width) || 450, height: Number(s.height) || 600 });
        }
        const css = s.css && s.css.trim() !== OBS_DEFAULT_CSS ? `${MS.appInfo.defaultBrowserCss}\n${s.css}` : MS.appInfo.defaultBrowserCss;
        return mk('browser', { url: url || '', width: Number(s.width) || 800, height: Number(s.height) || 600, fps: s.fps_custom ? Number(s.fps) || 30 : 30, css });
      }
      case 'game_capture':
        // Mode « n'importe quelle application en plein écran » : même principe que la Capture de jeu de MyStream.
        if (s.capture_mode !== 'window') {
          notes.push({ level: 'info', text: `« ${obs.name} » : capture de jeu automatique (le jeu en plein écran ou fenêtré sans bordure est capturé tout seul).` });
          return mk('game_capture', { captureAudio: s.capture_audio !== false });
        }
      // falls through : capture de jeu d'une fenêtre précise
      case 'window_capture': {
        const w = parseWindow(s.window);
        const game = obs.id === 'game_capture';
        if (!w.title && !w.exe) {
          notes.push({ level: 'todo', text: `« ${obs.name} » : capture de fenêtre sans fenêtre mémorisée, convertie en capture de jeu automatique.` });
          return mk('game_capture', { captureAudio: Boolean(s.capture_audio) });
        }
        if (game) notes.push({ level: 'info', text: `« ${obs.name} » : capture de jeu convertie en capture de la fenêtre « ${w.title || w.exe} ». Elle démarre dès que le jeu est lancé. Si l'image reste noire, passez le jeu en plein écran fenêtré (sans bordure).` });
        return mk('display_capture', { captureKind: 'window', captureName: w.title, captureAudio: Boolean(s.capture_audio), windowMatch: { title: w.title, exe: w.exe } });
      }
      case 'monitor_capture':
      case 'xshm_input':
      case 'screen_capture':
      case 'pipewire-desktop-capture-source': {
        const idx = Number(s.monitor ?? s.screen ?? 0) || 0;
        return mk('display_capture', { captureKind: 'screen', captureId: `screen:${idx}:0`, captureAudio: false });
      }
      case 'dshow_input':
      case 'av_capture_input':
      case 'av_capture_input_v2':
      case 'v4l2_input': {
        const raw = s.video_device_id || s.device_name || s.device || '';
        const label = String(raw).split(':')[0].replace(/#3A/g, ':').replace(/#22/g, '#').trim();
        const res = s.res_type === 0 ? 'default' : s.resolution && /^\d+x\d+$/.test(s.resolution) ? s.resolution : 'default';
        const fps = s.frame_interval > 0 ? Math.round(1e7 / s.frame_interval) : 0;
        if (label) notes.push({ level: 'info', text: `« ${obs.name} » : caméra « ${label} » retrouvée par son nom au démarrage.` });
        return mk('video_capture', { deviceId: '', deviceLabel: label, resolution: res, fps: fps >= 55 ? '60' : fps >= 25 && fps <= 35 ? '30' : 'default' });
      }
      case 'wasapi_input_capture':
      case 'pulse_input_capture':
      case 'coreaudio_input_capture':
        if (s.device_id && s.device_id !== 'default') notes.push({ level: 'todo', text: `« ${obs.name} » : micro réglé sur le périphérique par défaut : choisissez le bon dans ses propriétés.` });
        return mk('audio_input', { deviceId: 'default' });
      case 'wasapi_output_capture':
      case 'pulse_output_capture':
      case 'coreaudio_output_capture':
        return mk('desktop_audio', {});
      case 'wasapi_process_output_capture': {
        const w = parseWindow(s.window);
        notes.push({ level: 'info', text: `« ${obs.name} » : son de l'application « ${w.exe || w.title} » (Capture audio d'application), retrouvée même si son titre change.` });
        return mk('app_audio', { captureKind: 'window', captureName: w.title, windowMatch: { title: w.title, exe: w.exe } });
      }
      case 'image_source':
        return mk('image', { file: s.file || '' });
      case 'slideshow': {
        const files = (s.files || []).map((f) => f.value).filter(Boolean);
        if (files.length > 1) notes.push({ level: 'todo', text: `« ${obs.name} » : diaporama de ${files.length} images, seule la première est reprise.` });
        return mk('image', { file: files[0] || '' });
      }
      case 'ffmpeg_source': {
        if (s.is_local_file === false) {
          notes.push({ level: 'todo', text: `« ${obs.name} » : média en ligne (${s.input || 'URL'}) non pris en charge, source ignorée.` });
          return null;
        }
        const file = s.local_file || '';
        if (IMAGE_EXT.includes(ext(file))) {
          if (ext(file) === 'gif') ctx.gifs.push(obs.name);
          return mk('image', { file });
        }
        return mk('media', { file, loop: s.looping !== false && Boolean(s.looping) });
      }
      case 'vlc_source': {
        const first = (s.playlist || []).map((p) => p.value).find(Boolean);
        if (!first) return null;
        notes.push({ level: 'info', text: `« ${obs.name} » : liste VLC, seul le premier fichier est lu.` });
        return mk('media', { file: first, loop: s.loop !== false });
      }
      case 'color_source':
      case 'color_source_v2':
      case 'color_source_v3': {
        const c = obsColor(s.color, 0xffd1d1d1);
        return mk('color', { color: c.hex, width: Number(s.width) || ctx.width, height: Number(s.height) || ctx.height });
      }
      case 'text_gdiplus':
      case 'text_gdiplus_v2':
      case 'text_gdiplus_v3':
      case 'text_ft2_source':
      case 'text_ft2_source_v2': {
        const font = s.font || {};
        const flags = Number(font.flags) || 0;
        const ft2 = obs.id.startsWith('text_ft2');
        const color = obsColor(ft2 ? s.color1 : s.color);
        const outlineColor = obsColor(s.outline_color, 0xff000000);
        const bk = obsColor(s.bk_color, 0xff000000);
        if (s.read_from_file) notes.push({ level: 'todo', text: `« ${obs.name} » : texte lu depuis un fichier (${baseName(s.file)}) non pris en charge, texte fixe repris.` });
        return mk('text', {
          text: s.text || '',
          font: font.face || 'Arial',
          size: Number(font.size) || 36,
          bold: Boolean(flags & 1),
          italic: Boolean(flags & 2),
          color: color.hex,
          align: ['left', 'center', 'right'].includes(s.align) ? s.align : 'left',
          outline: Boolean(s.outline),
          outlineColor: outlineColor.hex,
          outlineSize: Number(s.outline_size) || 2,
          bgColor: bk.hex,
          bgOpacity: ft2 ? 0 : Number(s.bk_opacity) || 0,
        });
      }
      default:
        return null;
    }
  }

  /* Transitions --------------------------------------------------------------- */

  function convertTransition(obs, duration) {
    const s = (obs && obs.settings) || {};
    const dirs = { left: 'left', right: 'right', up: 'up', down: 'down' };
    switch (obs && obs.id) {
      case 'cut_transition':
        return { type: 'cut', duration };
      case 'swipe_transition':
      case 'slide_transition':
        return { type: 'slide', duration, direction: dirs[s.direction] || 'left' };
      case 'obs_stinger_transition':
        return {
          type: 'stinger',
          duration,
          stingerFile: s.path || '',
          stingerPoint: s.transition_point != null && Number(s.tp_type || 0) === 0 ? Number(s.transition_point) : null,
          stingerFrame: s.transition_point != null && Number(s.tp_type || 0) === 1 ? Number(s.transition_point) : null,
          stingerAudio: true,
          name: obs.name,
        };
      default:
        return { type: 'fade', duration };
    }
  }

  /** Point de changement de scène d'un stinger : en images (GIF) ou absent dans OBS. */
  async function resolveStingerPoint(t, notes) {
    if (t.stingerPoint != null) return t.stingerPoint;
    if (MS.isGif(t.stingerFile) && 'ImageDecoder' in window) {
      try {
        const gif = await new MS.GifPlayer(MS.fileUrl(t.stingerFile)).load();
        const point = t.stingerFrame != null ? gif.timeOfFrame(t.stingerFrame) : Math.round(gif.duration / 2);
        if (t.stingerFrame == null) notes.push({ level: 'info', text: `Stinger « ${t.name} » : point de changement non défini dans OBS, réglé au milieu de l'animation (${point} ms, modifiable dans Options… du dock Transitions).` });
        gif.close();
        return point;
      } catch {
        notes.push({ level: 'todo', text: `Stinger « ${t.name} » : fichier introuvable (${t.stingerFile}).` });
      }
    }
    if (t.stingerFrame != null) {
      notes.push({ level: 'todo', text: `Stinger « ${t.name} » : point de changement en images converti en supposant 30 i/s, à vérifier.` });
      return Math.round((t.stingerFrame / 30) * 1000);
    }
    return 600;
  }

  /* Conversion complète --------------------------------------------------------- */

  /**
   * Convertit la collection OBS. Ne modifie rien : renvoie les scènes et
   * sources MyStream à importer, les réglages à appliquer et les remarques.
   */
  MS.convertObsCollection = async function (data, options = {}) {
    const notes = [];
    const res = data.resolution || {};
    const width = Number(res.x) || MS.store.settings.video.width;
    const height = Number(res.y) || MS.store.settings.video.height;
    const ctx = { notes, options, width, height, gifs: [] };
    const byUuid = new Map();
    const byName = new Map();
    const obsByName = new Map(data.sources.map((o) => [o.name, o]));
    const sources = {};
    const skipped = new Set();

    for (const obs of data.sources) {
      if (obs.id === 'scene' || obs.id === 'group') continue;
      const src = convertSource(obs, ctx);
      if (!src) {
        if (!['ffmpeg_source', 'vlc_source'].includes(obs.id)) notes.push({ level: 'todo', text: `« ${obs.name} » : source « ${obs.id} » sans équivalent dans MyStream, ignorée.` });
        skipped.add(obs.name);
        continue;
      }
      convertFilters(obs, src, notes);
      sources[src.id] = src;
      if (obs.uuid) byUuid.set(obs.uuid, src);
      byName.set(obs.name, src);
    }
    if (ctx.gifs.length) notes.push({ level: 'info', text: `GIF animés repris en images animées : ${ctx.gifs.map((n) => `« ${n} »`).join(', ')}.` });

    // Sources audio globales d'OBS (Mic/Aux, Audio du bureau).
    const globals = [];
    const globalKeys = ['DesktopAudioDevice1', 'DesktopAudioDevice2', 'AuxAudioDevice1', 'AuxAudioDevice2', 'AuxAudioDevice3', 'AuxAudioDevice4'];
    for (const key of globalKeys) {
      const obs = data[key];
      if (!obs || obs.enabled === false) continue;
      const desktop = key.startsWith('Desktop');
      const main = key === 'AuxAudioDevice1' || key === 'DesktopAudioDevice1';
      const id = key === 'AuxAudioDevice1' ? 'global_mic' : key === 'DesktopAudioDevice1' ? 'global_desktop' : `global_${key.toLowerCase()}`;
      const src = {
        id,
        type: desktop ? 'desktop_audio' : 'audio_input',
        name: obs.name || (desktop ? 'Audio du bureau' : 'Mic/Aux'),
        settings: desktop ? {} : { deviceId: 'default', noiseSuppression: false },
        volume: obs.volume == null ? 1 : Number(obs.volume),
        muted: Boolean(obs.muted),
        monitor: Number(obs.monitoring_type) > 0,
        global: true,
      };
      convertFilters(obs, src, notes);
      globals.push({ key, src, main });
      if (!main) notes.push({ level: 'todo', text: `« ${src.name} » : source audio globale supplémentaire réglée sur le périphérique par défaut, à choisir dans ses propriétés.` });
    }

    // Scènes.
    const sceneByName = new Map(data.sources.filter((o) => o.id === 'scene').map((o) => [o.name, o]));
    const groupByName = new Map((data.groups || []).map((g) => [g.name, g]));
    const crops = new Map();
    const scenes = [];
    let rotated = 0;
    const order = data.scene_order.map((s) => s.name).filter((n) => sceneByName.has(n));

    const placeItem = (it, parent) => {
      const ref = it.scale_ref || { x: width, y: height };
      const f = width / (Number(ref.x) || width);
      let pos = { x: Number(it.pos && it.pos.x) || 0, y: Number(it.pos && it.pos.y) || 0 };
      let scale = { x: Number(it.scale && it.scale.x) || 1, y: Number(it.scale && it.scale.y) || 1 };
      if (parent) {
        pos = { x: parent.pos.x + pos.x * parent.scale.x, y: parent.pos.y + pos.y * parent.scale.y };
        scale = { x: scale.x * parent.scale.x, y: scale.y * parent.scale.y };
      }
      if (Number(it.rot)) rotated++;
      const align = it.align == null ? 5 : Number(it.align);
      const boundsType = Number(it.bounds_type) || 0;
      if (align === 5 && !boundsType) return { x: Math.round(pos.x * f), y: Math.round(pos.y * f), scaleX: scale.x * f, scaleY: scale.y * f };
      return { x: 0, y: 0, scaleX: null, scaleY: null, obsPlace: { pos, scale, align, boundsType, bounds: it.bounds || { x: 0, y: 0 }, boundsAlign: Number(it.bounds_align) || 0, f } };
    };

    const addItems = (scene, items, parent, sceneName) => {
      for (const it of items) {
        const obsSrc = (it.source_uuid && data.sources.find((o) => o.uuid === it.source_uuid)) || obsByName.get(it.name);
        const group = groupByName.get(it.name);
        if (group && (!obsSrc || obsSrc.id === 'group')) {
          // Groupe : ses éléments sont posés directement dans la scène.
          const gp = { pos: { x: Number(it.pos.x) || 0, y: Number(it.pos.y) || 0 }, scale: { x: Number(it.scale.x) || 1, y: Number(it.scale.y) || 1 } };
          addItems(scene, (group.settings && group.settings.items) || [], gp, sceneName);
          notes.push({ level: 'info', text: `Scène « ${sceneName} » : groupe « ${it.name} » dégroupé (MyStream n'a pas de groupes).` });
          continue;
        }
        if (obsSrc && obsSrc.id === 'scene') {
          notes.push({ level: 'todo', text: `Scène « ${sceneName} » : la scène imbriquée « ${it.name} » n'est pas prise en charge, à reconstruire.` });
          continue;
        }
        const src = (it.source_uuid && byUuid.get(it.source_uuid)) || byName.get(it.name);
        if (!src) continue;
        const crop = { cropLeft: Number(it.crop_left) || 0, cropTop: Number(it.crop_top) || 0, cropRight: Number(it.crop_right) || 0, cropBottom: Number(it.crop_bottom) || 0 };
        if (crop.cropLeft || crop.cropTop || crop.cropRight || crop.cropBottom) {
          const prev = crops.get(src.id);
          if (!prev) {
            crops.set(src.id, crop);
            src.filters = { ...MS.filterDefaults(), ...(src.filters || {}), ...crop };
          } else if (JSON.stringify(prev) !== JSON.stringify(crop)) {
            notes.push({ level: 'todo', text: `« ${src.name} » est recadrée différemment selon les scènes : seul le premier recadrage est gardé.` });
          }
        }
        const item = { id: MS.uid(), sourceId: src.id, visible: it.visible !== false, locked: Boolean(it.locked), ...placeItem(it, parent) };
        // OBS liste du fond vers le premier plan ; MyStream du premier plan vers le fond.
        scene.items.unshift(item);
      }
    };

    for (const name of order) {
      const obs = sceneByName.get(name);
      const scene = { id: MS.uid(), name, items: [] };
      addItems(scene, (obs.settings && obs.settings.items) || [], null, name);
      // Transition propre à la scène (« Remplacement de transition » d'OBS).
      const ps = obs.private_settings || {};
      if (ps.transition) {
        const t = (data.transitions || []).find((x) => x.name === ps.transition);
        const conv = t ? convertTransition(t, Number(ps.transition_duration) || 300) : convertTransition({ id: /coupure|cut/i.test(ps.transition) ? 'cut_transition' : 'fade_transition' }, Number(ps.transition_duration) || 300);
        if (conv.type === 'stinger') conv.stingerPoint = await resolveStingerPoint(conv, notes);
        delete conv.stingerFrame;
        delete conv.name;
        scene.transition = conv;
      }
      scenes.push(scene);
    }
    if (rotated) notes.push({ level: 'todo', text: `${rotated} élément(s) pivoté(s) dans OBS : la rotation n'existe pas encore dans MyStream, ils sont droits.` });

    // Transition active et stinger éventuel.
    const duration = Number(data.transition_duration) || 300;
    const custom = (data.transitions || []).find((t) => t.name === data.current_transition);
    const transition = custom ? convertTransition(custom, duration) : convertTransition({ id: /^(coupure|cut|schnitt|corte)$/i.test(data.current_transition || '') ? 'cut_transition' : 'fade_transition' }, duration);
    const stinger = transition.type === 'stinger' ? transition : (data.transitions || []).map((t) => convertTransition(t, duration)).find((t) => t.type === 'stinger');
    if (stinger) {
      stinger.stingerPoint = await resolveStingerPoint(stinger, notes);
      if (transition.type !== 'stinger') notes.push({ level: 'info', text: `Stinger « ${stinger.name} » prêt : choisissez « Stinger » dans le dock Transitions pour l'utiliser.` });
      Object.assign(transition, { stingerFile: stinger.stingerFile, stingerPoint: stinger.stingerPoint, stingerAudio: true });
    }
    delete transition.stingerFrame;
    delete transition.name;
    const labels = { fade: 'Fondu', cut: 'Coupure', slide: 'Glissement', stinger: 'Stinger' };
    notes.unshift({ level: 'info', text: `Transition : ${labels[transition.type]}${transition.type === 'cut' || transition.type === 'stinger' ? '' : ` ${transition.duration} ms`}.` });

    return {
      name: data.name || 'OBS',
      scenes,
      sources,
      globals,
      transition,
      video: { width, height },
      currentScene: data.current_program_scene || data.current_scene,
      skipped: [...skipped],
      notes,
    };
  };

  /* Interface ----------------------------------------------------------------- */

  MS.importObsCollection = async function (preloaded) {
    let data = preloaded;
    if (!data) {
      try {
        data = await window.api.importConfig('obs');
      } catch (e) {
        return MS.toast(`Fichier illisible : ${e.message}`, 'error', 8000);
      }
      if (!data) return;
    }
    if (!MS.isObsCollection(data)) return MS.toast("Ce fichier n'est pas une collection de scènes OBS (dans OBS : Collection de scènes > Exporter).", 'error', 8000);

    const st = MS.store.settings;
    const res = data.resolution || {};
    const resDiffers = res.x && res.y && (res.x !== st.video.width || res.y !== st.video.height);
    const chatWidgets = data.sources.filter((o) => o.id === 'browser_source' && CHAT_WIDGETS.test(String((o.settings || {}).url || '')));
    const sceneCount = data.scene_order.length;
    const sourceCount = data.sources.filter((o) => o.id !== 'scene' && o.id !== 'group').length;

    const fields = [
      { type: 'info', text: `« ${data.name || 'Collection OBS'} » : ${sceneCount} scène(s), ${sourceCount} source(s). Vos réglages MyStream (stream, chat, Restream, docks, raccourcis) ne sont pas modifiés.` },
      resDiffers ? { key: 'resolution', type: 'checkbox', label: `Passer le canevas en ${res.x}×${res.y} comme dans OBS (actuellement ${st.video.width}×${st.video.height})` } : null,
      chatWidgets.length ? { key: 'replaceChat', type: 'checkbox', label: `Remplacer ${chatWidgets.length > 1 ? 'les widgets' : 'le widget'} de chat (${chatWidgets.map((o) => o.name).join(', ')}) par la source « Chat du stream » de MyStream (Twitch + Kick)` } : null,
      { key: 'mode', type: 'select', label: 'Scènes actuelles', options: [{ value: 'add', label: 'Les garder et ajouter celles d’OBS' }, { value: 'replace', label: 'Les remplacer par celles d’OBS (Ctrl+Z pour revenir)' }] },
    ].filter(Boolean);
    const form = MS.buildForm(fields, { resolution: true, replaceChat: false, mode: 'add' });
    const ok = await MS.modal({ title: 'Importer une collection OBS', content: form.element, width: 560, buttons: [{ label: 'Annuler', value: false }, { label: 'Importer', value: true, primary: true }] });
    if (!ok) return;
    const opt = form.values();
    if (MS.outputs.isAnyActive() && opt.resolution && resDiffers) {
      opt.resolution = false;
      MS.toast('Résolution non modifiée : un streaming ou un enregistrement est en cours.', 'info', 6000);
    }

    const conv = await MS.convertObsCollection(data, { replaceChat: Boolean(opt.replaceChat) });
    if (!conv.scenes.length) return MS.toast('Aucune scène à importer dans ce fichier.', 'error', 6000);

    // Sources globales : on garde celles de MyStream (et leur périphérique), avec le volume et les filtres d'OBS.
    const d = MS.store.data;
    const current = JSON.parse(JSON.stringify(d.sources));
    const globals = Object.fromEntries(Object.entries(current).filter(([, s]) => s.global));
    for (const { key, src, main } of conv.globals) {
      const existing = globals[src.id];
      if (existing && main) {
        Object.assign(existing, { volume: src.volume, muted: src.muted, monitor: src.monitor, name: src.name });
        if (src.audioFilters) existing.audioFilters = src.audioFilters;
        if (src.settings.noiseSuppression) existing.settings.noiseSuppression = true;
      } else if (!existing) {
        globals[src.id] = src;
      }
      if (key === 'AuxAudioDevice1' && st.audio.micDevice === 'disabled') st.audio.micDevice = 'default';
      if (key === 'DesktopAudioDevice1') st.audio.desktopAudio = true;
    }

    let scenes;
    let sources;
    if (opt.mode === 'replace') {
      scenes = conv.scenes;
      sources = { ...globals, ...conv.sources };
    } else {
      const taken = d.scenes.map((s) => s.name);
      for (const s of conv.scenes) {
        s.name = MS.store.uniqueName(s.name, taken);
        taken.push(s.name);
      }
      const names = Object.values(current).map((s) => s.name);
      for (const s of Object.values(conv.sources)) {
        s.name = MS.store.uniqueName(s.name, names);
        names.push(s.name);
      }
      scenes = [...JSON.parse(JSON.stringify(d.scenes)), ...conv.scenes];
      sources = { ...current, ...globals, ...conv.sources };
    }

    Object.assign(st.transition, conv.transition);
    if (opt.resolution && resDiffers) {
      Object.assign(st.video, conv.video);
      MS.outputs.resetStream();
      MS.compositor.applyVideoSettings();
    }
    MS.history.replaceAll(scenes, sources);
    const live = conv.scenes.find((s) => s.name === conv.currentScene) || conv.scenes[0];
    MS.compositor.transition = null;
    MS.store.setCurrentScene(live.id);
    if (st.ui.studio) MS.store.setPreviewScene(live.id);
    MS.store.save();
    MS.syncGlobalAudio();
    MS.compositor.preloadStingers();
    MS.emit('settings:changed');
    MS.emit('transition:changed');
    showReport(conv, opt);
  };

  function showReport(conv, opt) {
    const todo = conv.notes.filter((n) => n.level === 'todo');
    const info = conv.notes.filter((n) => n.level === 'info');
    const list = (items) => el('ul', { class: 'import-notes' }, items.map((n) => el('li', { text: n.text })));
    const content = el(
      'div',
      { class: 'import-report' },
      el('p', { text: `${conv.scenes.length} scène(s) et ${Object.keys(conv.sources).length} source(s) importées${opt.mode === 'replace' ? ' (scènes précédentes remplacées, Ctrl+Z pour revenir)' : ', ajoutées à vos scènes'}.` }),
      todo.length ? el('h4', { text: `À vérifier (${todo.length})` }) : null,
      todo.length ? list(todo) : null,
      el('h4', { text: 'Adaptations faites' }),
      list(info)
    );
    MS.modal({ title: 'Collection OBS importée', content, width: 640, buttons: [{ label: 'OK', value: true, primary: true }] });
  }
})();
