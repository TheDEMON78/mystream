# MyStream natif (Rust)

Version native de MyStream (BloopStudio), écrite en Rust, sans Electron ni
navigateur intégré. Objectif : la fluidité et la légèreté d'OBS.

> **Préversion 0.1.** Elle pose les fondations (capture, composition,
> encodage, interface). Elle ne remplace pas encore la version Electron :
> voir « Ce qui manque encore ».

## Télécharger

Chaque envoi sur cette branche produit une préversion :
[Releases](https://github.com/TheDEMON78/mystream/releases) → « MyStream
natif 0.1.x (préversion) » → `MyStream-natif-setup.exe` (installation sans
droits administrateur, raccourcis sur le bureau et dans le menu Démarrer), ou
`MyStream-natif-win64.zip` à décompresser. FFmpeg est inclus.

## Pourquoi c'est plus léger

| Étape | Version Electron | Version native |
|---|---|---|
| Capture | Chromium : image copiée vers le processeur, convertie en I420, renvoyée à la carte graphique | Windows Graphics Capture (comme OBS), limitée à la cadence du stream |
| Composition | Canevas 2D de Chromium | Directement sur la carte graphique (wgpu : Direct3D 12 / Vulkan) |
| Cadence | Minuteurs du navigateur (arrondis à 15,6 ms en jeu) | Fil de rendu dédié, horloge haute résolution (~0,1 ms) |
| Conversion des couleurs | Processeur (FFmpeg) | Carte graphique : NV12 BT.709, le format natif de NVENC |
| Encodage | Deux fois : MediaRecorder puis FFmpeg (avec décodage entre les deux) | **Une seule fois**, FFmpeg (NVENC, Quick Sync, AMF ou x264) |
| Mémoire | ~2 à 3 Go (un processus Chromium par page web) | Quelques centaines de Mo |

## Fonctionnalités

- **Scènes** : ajouter, supprimer, renommer, transitions (coupure, fondu,
  glissement, **stinger** vidéo avec point de coupure), **mode studio** (aperçu modifiable + bouton Transition).
- **Sources** :
  - capture de jeu **automatique** (plein écran ou fenêtré sans bordure,
    gardée pendant un alt-tab), avec le son du jeu ;
  - capture d'écran, capture de fenêtre (avec son), caméra ;
  - image (GIF animés compris), couleur, texte, média (vidéo ou son, en
    boucle ; WebM transparents) ;
  - **navigateur** (alertes Streamlabs/StreamElements, widgets) : page web
    WebView2 avec sa transparence, CSS personnalisé et son propre son ;
  - son d'une application seule (Discord…), même en arrière-plan ;
  - source partagée entre plusieurs scènes.
- **Filtres** par source, sur la carte graphique : opacité, luminosité,
  contraste, saturation, recadrage, miroir, retournement, incrustation (fond vert).
- **Aperçu interactif** : clic pour sélectionner, glisser pour déplacer,
  molette pour la taille.
- **Mixeur** : micro, son du bureau et une piste par source sonore (volume,
  coupure, vumètre).
- **Streaming** RTMP et **enregistrement** MKV/MP4 simultanés, encodeur
  matériel détecté automatiquement.
- **Stream Deck** et autres télécommandes : serveur compatible
  **obs-websocket 5** (port 4455, mot de passe généré ; plugin MyStream ou
  plugins OBS, Touch Portal, Companion…).
- **Raccourcis clavier globaux** (même en jeu) : streaming, enregistrement,
  micro, son du bureau, transition, scène suivante/précédente.
- **Import des collections OBS** (scènes, sources, positions, recadrages, volumes).
- **Mises à jour automatiques** : vérifiées au démarrage puis toutes les
  30 minutes, installées en arrière-plan (jamais pendant un live) et
  appliquées au prochain lancement, ou tout de suite en un clic.
- **Sources cachées arrêtées** : une caméra, une capture ou un média qui n'est
  plus affiché est arrêté après 3 s (la caméra s'éteint), puis relancé.
- **Restream** (votre application) : adresse d'envoi automatique et
  vérification du serveur RTMP, dock « Titre et catégorie » (titre, catégorie
  avec recherche, spectateurs par plateforme, mise à jour partout).
- **Chat** comme la version Electron : Restream, ou Twitch et Kick en direct
  sans compte ; logos des plateformes, vraies images des badges, emotes,
  mentions surlignées, messages système, modération ; source « Chat »
  incrustée dans le stream (police, couleurs, contour, durée d'affichage).
- **Fin du live** : récapitulatif (durée, messages, pic de spectateurs, images
  perdues, débit moyen) et historique des lives.
- **Filtres audio** (gain, noise gate, compresseur) et **écoute au casque**
  (sans écho dans le son du bureau).
- **Édition** : annuler / rétablir (Ctrl+Z / Ctrl+Y), verrouillage des sources,
  transformation précise (Ctrl+E), menu des scènes (dupliquer avec sources
  partagées ou indépendantes), export / import de la collection, sauvegardes
  automatiques.

- **Interface** : docks masquables et déplaçables (rangée du bas, colonne de
  droite), dispositions enregistrées, barre « En direct », assistant de
  configuration, zone de notification, calage magnétique dans l'aperçu
  (Alt pour le désactiver), journaux et diagnostic de performance (Aide).

## Ce qui manque encore (par rapport à la version Electron)

1. Interaction avec les pages des sources navigateur (clics, saisie), pour
   se connecter à un service depuis la source.
2. Capture sans aucune copie vers le processeur (texture partagée directement
   avec wgpu) et encodage NVENC depuis la carte graphique.

## Développement

```
cargo run                                     # Linux / Windows
cargo build --release --target x86_64-pc-windows-gnu   # Windows depuis Linux
```

`MYSTREAM_FFMPEG` permet d'indiquer un autre FFmpeg.
`MYSTREAM_AUTOTEST=<secondes>` enregistre pendant la durée indiquée puis
quitte, en affichant les mesures de rendu (tests automatiques).

Organisation du code (`src/`) :

- `model.rs` : scènes, sources, réglages (JSON) ;
- `capture.rs` : captures (Windows Graphics Capture ; xcap hors Windows), image, couleur ;
- `capture/platform/browser.rs` : source navigateur (WebView2 hors écran, capture transparente, son).
- `compositor.rs` : composition et conversion NV12 sur la carte graphique, relecture asynchrone ;
- `engine.rs` : fil de rendu à cadence exacte, envoi aux sorties ;
- `audio.rs` : micro et son du bureau (WASAPI), mixage 48 kHz ;
- `output.rs` : FFmpeg (streaming, enregistrement, détection des encodeurs) ;
- `app.rs` : interface (egui).
