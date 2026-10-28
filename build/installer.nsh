; Pages personnalisées de l'installateur Windows (NSIS, inclus par electron-builder).
; Fichier en UTF-8 avec BOM : sans lui, NSIS lirait mal les accents.

!macro customWelcomePage
  !define MUI_WELCOMEPAGE_TITLE "Bienvenue dans MyStream"
  !define MUI_WELCOMEPAGE_TEXT "MyStream est un logiciel de streaming et d'enregistrement par BloopStudio.$\r$\n$\r$\nScènes, chat Twitch et Kick en direct, transitions, Restream et mises à jour automatiques.$\r$\n$\r$\nSi MyStream est ouvert, il sera fermé pendant l'installation. Vos scènes et vos réglages sont conservés.$\r$\n$\r$\nCliquez sur Suivant pour continuer."
  !insertmacro MUI_PAGE_WELCOME
!macroend

!define MUI_FINISHPAGE_TITLE "MyStream est installé"
!define MUI_FINISHPAGE_TEXT "MyStream est prêt. Un raccourci a été ajouté sur le bureau et dans le menu Démarrer.$\r$\n$\r$\nAu premier lancement, un assistant vous aide à choisir votre plateforme, la qualité vidéo et votre micro."
!define MUI_FINISHPAGE_RUN_TEXT "Lancer MyStream"
!define MUI_FINISHPAGE_LINK "Voir les nouveautés de cette version"
!define MUI_FINISHPAGE_LINK_LOCATION "https://github.com/TheDEMON78/mystream/releases"
