# Plugin Stream Deck pour MyStream

Plugin Stream Deck officiel de **MyStream** (BloopStudio) : il pilote MyStream directement, sans OBS ni plugin tiers.

## Installation

1. Téléchargez **[MyStream.streamDeckPlugin](https://github.com/TheDEMON78/mystream/raw/streamdeck-plugin/MyStream.streamDeckPlugin)**, puis double-cliquez dessus : le logiciel Stream Deck l'installe.
2. Dans MyStream : **Paramètres > Stream Deck**, vérifiez que la télécommande est activée et notez le mot de passe (bouton « Afficher »).
3. Dans le logiciel Stream Deck, glissez une action de la catégorie **MyStream** sur une touche. Dans ses réglages, ouvrez **Connexion à MyStream** : adresse `127.0.0.1`, port `4455`, mot de passe de MyStream, puis **Se connecter**. La connexion vaut pour toutes les touches.

## Actions

| Action | Effet | La touche s'allume… |
| --- | --- | --- |
| **Scène** | Passe la scène choisie à l'antenne (son nom s'affiche sur la touche) | quand la scène est à l'antenne |
| **Couper le son** | Coupe ou rétablit le micro, ou une autre source audio (Discord, musique…) | en rouge quand le son est coupé |
| **Afficher / masquer une source** | Affiche ou masque une source dans une scène | quand la source est affichée |
| **Streaming** | Démarre ou arrête le live | pendant le live, avec sa durée |
| **Enregistrement** | Démarre ou arrête l'enregistrement | pendant l'enregistrement, avec sa durée |
| **Transition (mode studio)** | Passe la scène de l'aperçu à l'antenne | — |

Les touches suivent aussi les changements faits dans MyStream (scène changée à la souris, micro coupé par un raccourci…). Si MyStream est fermé, la touche affiche un avertissement et le plugin se reconnecte tout seul dès qu'il revient.

## Fonctionnement

Le plugin parle à MyStream par le protocole obs-websocket v5, que MyStream intègre (Paramètres > Stream Deck). Il fonctionne sous Windows 10/11 et macOS, avec le logiciel Stream Deck 6 ou plus récent.

## Développement

- `com.bloopstudio.mystream.sdPlugin/` : le plugin (manifeste, `plugin.js` pour les touches, `pi.html` / `pi.js` pour les réglages d'une touche, icônes).
- `python3 tools/icons.py` : redessine les icônes.
- `python3 tools/pack.py` : recrée `MyStream.streamDeckPlugin`.

Pour tester sans repackager : copiez le dossier `com.bloopstudio.mystream.sdPlugin` dans `%APPDATA%\Elgato\StreamDeck\Plugins\` (Windows) ou `~/Library/Application Support/com.elgato.StreamDeck/Plugins/` (macOS), puis redémarrez le logiciel Stream Deck.
