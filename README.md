# MyStream

**MyStream** est un logiciel libre de streaming et d'enregistrement vidéo pour Windows, Linux et macOS, inspiré d'OBS Studio.
Composez vos scènes à partir de plusieurs sources (écran, webcam, images, vidéos, texte…), mixez l'audio et diffusez en direct sur Twitch, YouTube, Kick ou tout serveur RTMP, ou enregistrez en MP4/MKV/WebM.

## Fonctionnalités

- **Scènes** : création, renommage (double-clic), suppression, réorganisation, transitions *Fondu* ou *Coupure* avec durée réglable.
- **Sources** :
  - Capture d'écran / de fenêtre (sélecteur avec miniatures)
  - Périphérique de capture vidéo (webcam, carte d'acquisition) avec choix de résolution et de FPS
  - Capture d'entrée audio (micro) et audio du bureau (Windows)
  - Image, source média (vidéo/audio, lecture en boucle)
  - Texte (police, taille, couleur, contour, fond, alignement) et source de couleur
  - Réutilisation d'une source existante dans plusieurs scènes
- **Aperçu interactif** : sélection, déplacement, redimensionnement à la souris (poignées, `Maj` pour déformer, `Alt` pour désactiver l'aimantation), flèches du clavier, `Suppr`, menu contextuel (ajuster/étirer/centrer à l'écran, taille d'origine, ordre, verrouillage).
- **Mixeur audio** : VU-mètres, faders, muet, écoute (monitoring) par source ; sources globales *Mic/Aux* et *Audio du bureau*.
- **Streaming RTMP/RTMPS** : Twitch, YouTube, Kick, Facebook Live ou serveur personnalisé, encodage x264 + AAC via FFmpeg, débit affiché en direct.
- **Enregistrement** : MP4 (H.264/AAC), MKV ou WebM, dossier configurable.
- **Paramètres** : résolution du canevas (dont vertical 1080×1920), FPS, débits, préréglage x264, périphérique micro.
- **Barre d'état** : FPS de rendu, images en retard, CPU/mémoire, durée du live et de l'enregistrement.
- Configuration (scènes, sources, réglages) sauvegardée automatiquement.

## Téléchargement

Les versions compilées sont publiées automatiquement dans les [Releases](https://github.com/TheDEMON78/mystream/releases) :

| Système | Fichier |
| --- | --- |
| Windows | `MyStream-x.y.z-win-setup.exe` (installateur) ou `-win-portable.exe` |
| Linux | `.AppImage` ou `.deb` |
| macOS | `.dmg` (non signé : clic droit → *Ouvrir* au premier lancement) |

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
│   └── preload.js     # pont sécurisé (contextBridge) entre l'interface et le processus principal
├── renderer/
│   ├── index.html, styles.css
│   └── js/
│       ├── store.js       # modèle de données (scènes, sources, éléments, réglages)
│       ├── sources.js     # types de sources et leurs moteurs (webcam, écran, image, texte…)
│       ├── audio.js       # mixeur Web Audio et sources audio globales
│       ├── compositor.js  # rendu des scènes dans le canevas, transitions
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
