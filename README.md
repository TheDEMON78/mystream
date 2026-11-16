# MyStream natif (Rust)

Version native de MyStream (BloopStudio), écrite en Rust, sans Electron ni
navigateur intégré. Objectif : la fluidité et la légèreté d'OBS.

> **Préversion 0.1.** Elle pose les fondations (capture, composition,
> encodage, interface). Elle ne remplace pas encore la version Electron :
> voir « Ce qui manque encore ».

## Télécharger

Chaque envoi sur cette branche produit une préversion :
[Releases](https://github.com/TheDEMON78/mystream/releases) → « MyStream
natif 0.1.x (préversion) » → `MyStream-natif-win64.zip`. Décompressez-le et
lancez `MyStream.exe` (FFmpeg est inclus).

## Pourquoi c'est plus léger

| Étape | Version Electron | Version native |
|---|---|---|
| Capture | Chromium : image copiée vers le processeur, convertie en I420, renvoyée à la carte graphique | Windows Graphics Capture (comme OBS), limitée à la cadence du stream |
| Composition | Canevas 2D de Chromium | Directement sur la carte graphique (wgpu : Direct3D 12 / Vulkan) |
| Cadence | Minuteurs du navigateur (arrondis à 15,6 ms en jeu) | Fil de rendu dédié, horloge haute résolution (~0,1 ms) |
| Conversion des couleurs | Processeur (FFmpeg) | Carte graphique : NV12 BT.709, le format natif de NVENC |
| Encodage | Deux fois : MediaRecorder puis FFmpeg (avec décodage entre les deux) | **Une seule fois**, FFmpeg (NVENC, Quick Sync, AMF ou x264) |
| Mémoire | ~2 à 3 Go (un processus Chromium par page web) | Quelques centaines de Mo |

## Fonctionnalités de la 0.1

- Scènes : ajouter, supprimer, renommer (double-clic), passer d'une scène à l'autre.
- Sources : capture d'écran, capture de fenêtre (jeu ; recherchée toutes les
  2 s si elle n'est pas encore ouverte), image, couleur. Afficher/masquer,
  monter/descendre, retirer.
- Aperçu interactif : clic pour sélectionner, glisser pour déplacer, molette
  pour agrandir ou réduire ; « Ajuster à l'écran », « Taille d'origine ».
- Mixeur : micro et son du bureau, volume, coupure, vumètres.
- Streaming RTMP (Restream, Twitch, YouTube…) et enregistrement MKV/MP4,
  simultanés.
- Encodeur automatique : le meilleur encodeur matériel qui fonctionne
  réellement sur le PC (testé au démarrage), sinon x264.
- Barre d'état : images par seconde, temps de rendu, images manquées ou
  perdues, durée et débit du live ou de l'enregistrement.
- Projet enregistré automatiquement (`%APPDATA%\MyStream-natif\projet.json`).

## Ce qui manque encore (par rapport à la version Electron)

Par ordre de priorité proposé :

1. Capture sans aucune copie vers le processeur (texture Windows Graphics
   Capture partagée directement avec wgpu) et encodage NVENC depuis la carte
   graphique.
2. Capture de jeu automatique et son d'une application (Discord, jeu).
3. Sources texte, média (vidéo), caméra.
4. Transitions, mode studio, filtres (incrustation, recadrage, couleurs).
5. Sources navigateur (alertes) et chat : moteur web à intégrer (WebView2).
6. Connexion Restream, Stream Deck (obs-websocket), raccourcis globaux.
7. Import des scènes OBS, mises à jour automatiques, installateur.

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
- `compositor.rs` : composition et conversion NV12 sur la carte graphique, relecture asynchrone ;
- `engine.rs` : fil de rendu à cadence exacte, envoi aux sorties ;
- `audio.rs` : micro et son du bureau (WASAPI), mixage 48 kHz ;
- `output.rs` : FFmpeg (streaming, enregistrement, détection des encodeurs) ;
- `app.rs` : interface (egui).
