# MyStream

**MyStream** est un logiciel libre de streaming et d'enregistrement vidéo pour Windows, Linux et macOS, inspiré d'OBS Studio.
Composez vos scènes à partir de plusieurs sources (écran, webcam, images, vidéos, texte…), mixez l'audio et diffusez en direct sur Twitch, YouTube, Kick ou tout serveur RTMP, ou enregistrez en MP4/MKV/WebM.

## Fonctionnalités

- **Scènes** : création, renommage (double-clic), suppression, réorganisation, transitions *Fondu* ou *Coupure* avec durée réglable.
- **Mode studio** : aperçu et programme côte à côte, bouton *Transition* pour passer l'aperçu à l'antenne (double-clic sur une scène = à l'antenne).
- **Sources** :
  - Capture d'écran / de fenêtre (sélecteur avec miniatures)
  - Périphérique de capture vidéo (webcam, carte d'acquisition) avec choix de résolution et de FPS
  - Capture d'entrée audio (micro) et audio du bureau (Windows)
  - Image, source média (vidéo/audio, lecture en boucle)
  - Texte (police, taille, couleur, contour, fond, alignement) et source de couleur
  - **Chat du stream** incrusté à l'écran (logos Twitch/Kick, badges, disparition automatique des anciens messages)
  - **Navigateur** : n'importe quelle page web (alertes StreamElements/Streamlabs, widgets…), fond transparent, CSS personnalisé
  - Réutilisation d'une source existante dans plusieurs scènes
- **Aperçu interactif** : sélection, déplacement, redimensionnement à la souris (poignées, `Maj` pour déformer, `Alt` pour désactiver l'aimantation), flèches du clavier, `Suppr`, menu contextuel (ajuster/étirer/centrer à l'écran, taille d'origine, ordre, verrouillage).
- **Filtres par source** : opacité, luminosité, contraste, saturation, teinte, recadrage, miroir et **incrustation fond vert** (calculée sur la carte graphique).
- **Dock Chat** : chat Twitch + Kick en direct à côté de l'aperçu, via Restream ou en lecture directe (sans compte).
- **Mixeur audio** : VU-mètres, faders, muet, écoute (monitoring) par source ; sources globales *Mic/Aux* et *Audio du bureau*.
- **Streaming RTMP/RTMPS** : Twitch, YouTube, Kick, Facebook Live ou serveur personnalisé, débit affiché en direct.
- **Encodeur matériel** détecté automatiquement (NVIDIA NVENC, Intel Quick Sync, AMD AMF, Apple VideoToolbox), sinon x264.
- **Intégration [Restream](https://github.com/TheDEMON78/restream)** : choisir le service *Restream (BloopStudio)* suffit, MyStream récupère tout seul l'adresse et la clé d'ingestion, affiche l'état des relais Twitch/Kick dans la barre d'état et le chat agrégé dans le dock.
- **Enregistrement** : MP4 (H.264/AAC), MKV ou WebM, dossier configurable.
- **Paramètres** : résolution du canevas (dont vertical 1080×1920), FPS, débits, préréglage x264, périphérique micro.
- **Barre d'état** : FPS de rendu (cadence exacte, jamais au-dessus de la valeur demandée), images manquées, CPU/mémoire, durée du live et de l'enregistrement.
- **Menus et docks** : Fichier / Affichage / Docks / Aide, chaque dock peut être masqué.
- Configuration (scènes, sources, réglages) sauvegardée automatiquement.

## Téléchargement

Les versions compilées sont publiées automatiquement dans les [Releases](https://github.com/TheDEMON78/mystream/releases) :

| Système | Fichier |
| --- | --- |
| Windows | `MyStream-x.y.z-win-setup.exe` (installateur) ou `-win-portable.exe` |
| Linux | `.AppImage` ou `.deb` |
| macOS | `.dmg` (non signé : clic droit → *Ouvrir* au premier lancement) |

## Utiliser MyStream avec Restream

1. Lance Restream (`npm start` ou `start.bat`).
2. Dans MyStream : **Paramètres > Restream**, renseigne l'adresse de l'interface (`http://localhost:3050` par défaut) et le mot de passe si `WEB_PASSWORD` est défini, puis **Tester la connexion**.
3. **Paramètres > Stream** : choisis le service **Restream (BloopStudio)**.
4. **Paramètres > Chat** : choisis **Restream** pour afficher le chat Twitch + Kick (avec les vrais badges Twitch).
5. Démarre le streaming : Restream relaie le flux vers Twitch et Kick sans le réencoder.

Pour incruster le chat dans le stream, ajoute une source **Chat du stream** à ta scène.

### Restream sur un serveur (NAS, VPS…)

- **Adresse** : mets l'adresse publique de l'interface, ex. `https://bloop.myds.me:3050`.
- **Mot de passe** : celui de `WEB_PASSWORD` (indispensable si Restream est exposé sur internet).
- **Certificat auto-signé** (`RESTREAM_TLS=self-signed`) : laisse cochée l'option *Accepter un certificat HTTPS auto-signé*.
- **Port RTMP** : MyStream envoie le flux sur le **même hôte**, port RTMP de Restream (1935 par défaut). Ce port doit être ouvert/redirigé sur le routeur ou le pare-feu du serveur.
- **Adresse RTMP différente** (reverse proxy, port public redirigé, autre nom d'hôte) : renseigne le champ *Adresse RTMP*, ex. `rtmp://bloop.myds.me:19350`. La clé d'ingestion reste récupérée automatiquement.
- Le chat Twitch fonctionne partout ; le chat Kick nécessite que Restream soit joignable en HTTPS (voir le README de Restream).

## Développement

Prérequis : Node.js 22+.

```bash
npm install      # installe Electron, electron-builder et FFmpeg
npm start        # lance l'application
npm run check    # vérifie la syntaxe des sources
npm run dist     # génère les installateurs pour le système courant dans dist/
```

### Architecture

```
src/
├── main/
│   ├── main.js        # processus principal : fenêtre, configuration, capture d'écran, FFmpeg
│   ├── restream.js    # client Restream : adresse d'ingestion, état des relais, chat (SSE)
│   ├── browserSources.js # sources Navigateur rendues hors écran
│   ├── encoders.js    # détection des encodeurs matériels
│   └── preload.js     # pont sécurisé (contextBridge) entre l'interface et le processus principal
├── renderer/
│   ├── index.html, styles.css
│   └── js/
│       ├── store.js       # modèle de données (scènes, sources, éléments, réglages)
│       ├── sources.js     # types de sources et leurs moteurs (webcam, écran, image, texte…)
│       ├── audio.js       # mixeur Web Audio et sources audio globales
│       ├── chat.js        # chat : Restream, Twitch (IRC anonyme), Kick (Pusher)
│       ├── compositor.js  # rendu des scènes, cadence d'images, filtres, incrustation, mode studio
│       ├── preview.js     # interactions de l'aperçu (sélection, déplacement, redimensionnement)
│       ├── outputs.js     # enregistrement et streaming (MediaRecorder → FFmpeg)
│       ├── dialogs.js     # fenêtres modales, propriétés, paramètres
│       └── ui.js          # docks : scènes, sources, mixeur, transitions, commandes
└── assets/icon.png
```

Le canevas de composition est capturé (`captureStream`) avec le mixage audio, encodé par `MediaRecorder`, puis transmis à FFmpeg qui produit le fichier final ou le flux RTMP.

## Versions

Chaque push sur `main` déclenche le workflow GitHub Actions *Build & Release*, qui compile l'application pour Windows, Linux et macOS et publie une release.
Le numéro de version est **`1.Y.X`**, où `X` est le nombre de commits modulo 100 et `Y` le nombre de centaines de commits (101 commits → `1.1.1`).

## Licence

MIT
