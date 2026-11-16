; Installateur de MyStream natif (BloopStudio).
; Installé pour l'utilisateur courant (pas de droits administrateur), dans un
; dossier où MyStream peut se mettre à jour tout seul.
Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.1.0"
!endif

Name "MyStream natif"
OutFile "..\MyStream-natif-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\MyStream natif"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ABORTWARNING
!define MUI_ICON "..\assets\icon.ico"
!define MUI_UNICON "..\assets\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\MyStream.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Lancer MyStream"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "French"

Section "MyStream"
  SetOutPath "$INSTDIR"
  File "..\paquet\MyStream.exe"
  File "..\paquet\ffmpeg.exe"
  File "..\paquet\LISEZMOI.md"
  WriteUninstaller "$INSTDIR\Desinstaller.exe"
  CreateShortCut "$SMPROGRAMS\MyStream natif.lnk" "$INSTDIR\MyStream.exe"
  CreateShortCut "$DESKTOP\MyStream natif.lnk" "$INSTDIR\MyStream.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\MyStreamNatif" "DisplayName" "MyStream natif"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\MyStreamNatif" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\MyStreamNatif" "Publisher" "BloopStudio"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\MyStreamNatif" "DisplayIcon" "$INSTDIR\MyStream.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\MyStreamNatif" "UninstallString" '"$INSTDIR\Desinstaller.exe"'
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\MyStream.exe"
  Delete "$INSTDIR\MyStream.old.exe"
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffmpeg.old.exe"
  Delete "$INSTDIR\LISEZMOI.md"
  Delete "$INSTDIR\Desinstaller.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\MyStream natif.lnk"
  Delete "$DESKTOP\MyStream natif.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\MyStreamNatif"
SectionEnd
