; TA Reborn - NSIS 3 installer script (Unicode, Modern UI 2, 64-bit Windows 10 or later).
;
; Build with installer\build_installer.ps1: it compiles the release binaries, stages them in
; dist\TA Reborn\ and runs makensis with the defines below. To compile by hand after staging:
;   makensis /DAPP_VERSION=0.1.0 installer\ta_reborn.nsi
;
; Defines (makensis /DNAME=value):
;   APP_VERSION    version shown to users (required), e.g. 0.1.0 or 0.2.0-beta.1
;   APP_ARCH       payload CPU architecture: x64 (default) or arm64
;   STAGE_DIR      folder with the checked payload (default: ..\dist\stage-windows-<arch>\TA Reborn)
;   OUTPUT_FILE    setup file to write (default: <OUTPUT_DIR>\TA-Reborn-<version>-windows-<arch>-setup.exe)
;   OUTPUT_DIR     folder for the setup file when OUTPUT_FILE is not given (default: ..\dist)
;   APP_URL        optional web site for the Add/Remove Programs entry (URLInfoAbout)
;   TEST_PER_USER  test build only: runs without elevation, installs to the folder given with
;                  /D=, registers the uninstaller under HKCU and creates per-user shortcuts
;
; Setup command line: /S (silent), /DESKTOPSHORTCUT, /GAMEDIR=<original game folder>, /UPDATE
; (the launcher updating itself: over an installed version, only the progress shows and the
; launcher starts again when it is done), and /D=<install folder> (last, unquoted). Test builds
; also accept /NODETECT, which makes Setup behave as if the original game were not installed.
;
; TA Reborn ships no original game data. Setup looks for the user's own Total Annihilation
; install (GOG registry key, Steam, default folders) and records it for the launcher; if there
; is none it explains what is needed and offers the GOG and Steam store pages, a folder picker, or setting
; it up later in the launcher.

Unicode true
SetCompressor /SOLID /FINAL lzma
SetCompressorDictSize 64
ManifestDPIAware true
; A scalable Windows UI font keeps the wizard's text as crisp as its artwork.
SetFont "Segoe UI" 9

;--------------------------------------------------------------------------------------------
; Product

; Only remove DLLs included by this installer; never delete arbitrary *.dll files.
; Keep this list aligned with MsvcRuntimeNames in windows_package.ps1.
!macro DeleteBundledRuntime NAME
  !if /FileExists "${STAGE_DIR}\${NAME}"
    Delete "$INSTDIR\${NAME}"
  !endif
!macroend

!define APP_NAME "TA Reborn"
!define APP_PUBLISHER "TA Reborn contributors"
!define APP_COMMENTS "Unofficial engine reimplementation. Requires a legitimately owned copy of Total Annihilation."
!define LAUNCHER_EXE "ta-launcher.exe"
!define GAME_EXE "ta-reborn.exe"
!define UNINSTALL_EXE "Uninstall.exe"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
!define SM_DIR "$SMPROGRAMS\${APP_NAME}"
!define DESKTOP_LNK "$DESKTOP\${APP_NAME}.lnk"
; The game's name up to 1.9, when it was TA Remastered. An installation under that name is
; replaced: its own uninstaller removes it (keeping the user's files, which the game moves to
; the new name when it first starts), and this version goes beside it under the new name.
!define OLD_APP_NAME "TA Remastered"
!define OLD_UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${OLD_APP_NAME}"

; The original game, located like the launcher does it (crates/ta_vfs/src/install.rs).
!define GOG_GAME_KEY "SOFTWARE\GOG.com\Games\1207658880"
!define STEAM_GAME_KEY "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 298030"
!define GOG_STORE_URL "https://www.gog.com/game/total_annihilation_commander_pack"
!define STEAM_STORE_URL "https://store.steampowered.com/app/298030/"
; Steam itself, where its default library is (the game in <Steam>\steamapps\common).
!define STEAM_KEY "SOFTWARE\Valve\Steam"

!ifndef APP_VERSION
  !error "APP_VERSION is not defined: compile with /DAPP_VERSION=x.y.z (build_installer.ps1 does this)."
!endif
!define /ifndef APP_ARCH "x64"
!if "${APP_ARCH}" != "x64"
!if "${APP_ARCH}" != "arm64"
  !error "APP_ARCH must be x64 or arm64."
!endif
!endif
; Windows version resources need a purely numeric a.b.c.d version: drop any "-pre" or "+build"
; suffix and pad with zeros.
!searchparse /noerrors "${APP_VERSION}" "" _TA_VERSION_NOPRE "-"
!searchparse /noerrors "${_TA_VERSION_NOPRE}" "" _TA_VERSION_CORE "+"
!searchparse /noerrors "${_TA_VERSION_CORE}" "" _TA_VERSION_1 "." _TA_VERSION_2 "." _TA_VERSION_3 "." _TA_VERSION_4
!define /ifndef _TA_VERSION_2 0
!define /ifndef _TA_VERSION_3 0
!define /ifndef _TA_VERSION_4 0
!define APP_VERSION_NUMERIC "${_TA_VERSION_1}.${_TA_VERSION_2}.${_TA_VERSION_3}.${_TA_VERSION_4}"

!ifndef STAGE_DIR
  !define STAGE_DIR "${__FILEDIR__}\..\dist\stage-windows-${APP_ARCH}\${APP_NAME}"
!endif
!ifndef OUTPUT_FILE
  !ifndef OUTPUT_DIR
    !define OUTPUT_DIR "${__FILEDIR__}\..\dist"
  !endif
  !define OUTPUT_FILE "${OUTPUT_DIR}\TA-Reborn-${APP_VERSION}-windows-${APP_ARCH}-setup.exe"
!endif
!if ! /FileExists "${STAGE_DIR}\${LAUNCHER_EXE}"
  !error "${STAGE_DIR}\${LAUNCHER_EXE} not found: stage the files first (build_installer.ps1 -StageOnly)."
!endif
!if ! /FileExists "${STAGE_DIR}\${GAME_EXE}"
  !error "${STAGE_DIR}\${GAME_EXE} not found: stage the files first (build_installer.ps1 -StageOnly)."
!endif

!ifdef TEST_PER_USER
  ; Never release this variant: it exists to test the installer without elevation, in a folder
  ; of the tester's choice.
  !define UNINST_ROOT HKCU
  !define SHELL_CONTEXT current
  !define DISPLAY_NAME "${APP_NAME} (per-user test build)"
  ; No InstallDir: $INSTDIR stays empty in .onInit unless /D= is given.
  !define DEFAULT_INSTDIR ""
  RequestExecutionLevel user
!else
  !define UNINST_ROOT HKLM
  !define SHELL_CONTEXT all
  !define DISPLAY_NAME "${APP_NAME}"
  !define DEFAULT_INSTDIR "$PROGRAMFILES64\${APP_NAME}"
  RequestExecutionLevel admin
  InstallDir "${DEFAULT_INSTDIR}"
!endif

Name "${DISPLAY_NAME}"
OutFile "${OUTPUT_FILE}"

; Code signing (build_installer.ps1 -SignScript): the uninstaller and the setup are signed by the
; same script as the programs, once each is written.
!ifdef SIGN_SCRIPT
  !uninstfinalize '"${SIGN_SCRIPT}" "%1"' = 0
  !finalize '"${SIGN_SCRIPT}" "%1"' = 0
!endif
BrandingText "${APP_NAME} ${APP_VERSION} (${APP_ARCH})"

!include MUI2.nsh
!include nsDialogs.nsh
!include LogicLib.nsh
!include x64.nsh
!include WinVer.nsh
!ifndef IsNativeARM64
  !error "Use a current NSIS 3 release with ARM64 host detection (x64.nsh)."
!endif
!ifndef AtLeastWin11
  !error "Use a current NSIS 3 release with Windows 11 detection (WinVer.nsh)."
!endif
!include FileFunc.nsh
!include Sections.nsh
!include artwork.nsh

; Folder recorded in config.toml; "" leaves the original game to the launcher.
Var GameDir
; State of the "Original game" page, kept while the user goes back and forth.
Var GameDirText
Var GameChoice
Var GameFound
Var GameDirField
Var GameBrowseButton
Var GameUseRadio
Var GameLaterRadio
Var GameStatusLabel
Var BoldFont
; Folder of an existing installation (upgrade), or "".
Var PrevInstDir
; Folder of an installation under the game's old name, to replace, or "".
Var OldInstDir
Var FinishText
; 1 = /UPDATE over an installed version: the launcher is updating itself.
Var Updating
Var HeaderBitmap
!ifdef TEST_PER_USER
  ; 1 = /NODETECT: behave as if the original game were not installed.
  Var NoDetect
!endif

;--------------------------------------------------------------------------------------------
; Pages

!define MUI_ABORTWARNING
; TA Reborn's own art (branding\, drawn by `tools/branding/emblem.py` and `ta-tools brand`):
; the emblem as the setup and uninstaller icon, and 4x-resolution gunmetal panels. The show
; callbacks resample from these masters at the control's actual pixel size with HALFTONE.
!define MUI_ICON "..\branding\ta-reborn.ico"
!define MUI_UNICON "..\branding\ta-reborn.ico"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "..\branding\installer-header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "..\branding\installer-header.bmp"
!define MUI_WELCOMEFINISHPAGE_BITMAP "..\branding\installer-welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "..\branding\installer-welcome.bmp"
!define MUI_CUSTOMFUNCTION_GUIINIT SetupArtwork
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.SetupArtwork
!define MUI_BGCOLOR "11151A"
!define MUI_TEXTCOLOR "DCE2E8"
; Windows draws a themed check box's text in the theme's own colour, whatever SetCtlColors asks
; (NSIS bug #443): the finish page's "Launch" box read black on the dark page. Unthemed, it takes
; the light text colour (MUI2 only changes the finish page's check boxes for this).
!define MUI_FORCECLASSICCONTROLS
!define MUI_WELCOMEPAGE_TITLE "Welcome to ${APP_NAME}"
!define MUI_WELCOMEPAGE_TEXT "Setup will install ${APP_NAME} ${APP_VERSION} on your computer.$\r$\n$\r$\nAn unofficial, clean-room reimplementation of the Total Annihilation engine.$\r$\n$\r$\nYou will need your own copy of Total Annihilation, such as Commander Pack from GOG.com or Steam. Setup will help you find it.$\r$\n$\r$\nNo original game data is included.$\r$\n$\r$\n$_CLICK"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenUpdating
!define MUI_PAGE_CUSTOMFUNCTION_SHOW WelcomeArtwork
!insertmacro MUI_PAGE_WELCOME
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenUpdating
!insertmacro MUI_PAGE_COMPONENTS
!define MUI_PAGE_CUSTOMFUNCTION_PRE DirectoryPagePre
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE DirectoryPageLeave
!insertmacro MUI_PAGE_DIRECTORY
Page custom GamePageCreate GamePageLeave
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TEXT "$FinishText"
!define MUI_FINISHPAGE_TEXT_LARGE
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${APP_NAME}"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchLauncher
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenUpdating
!define MUI_PAGE_CUSTOMFUNCTION_SHOW FinishArtwork
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

ReserveFile /plugin System.dll
ReserveFile /plugin nsDialogs.dll
!ifndef TEST_PER_USER
  ReserveFile /plugin UserInfo.dll
!endif

VIProductVersion "${APP_VERSION_NUMERIC}"
VIFileVersion "${APP_VERSION_NUMERIC}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileDescription" "${APP_NAME} ${APP_ARCH} Setup"
VIAddVersionKey /LANG=${LANG_ENGLISH} "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "LegalCopyright" "(c) ${APP_PUBLISHER}. Total Annihilation is (c) its respective owners."
VIAddVersionKey /LANG=${LANG_ENGLISH} "Comments" "${APP_COMMENTS}"

;--------------------------------------------------------------------------------------------
; Helpers shared by the installer and the uninstaller

Function SetupArtwork
  SendMessage $mui.Header.Image ${STM_GETIMAGE} ${IMAGE_BITMAP} 0 $HeaderBitmap
  !insertmacro TA_SMOOTH_BITMAP $mui.Header.Image "$PLUGINSDIR\modern-header.bmp" 600 228 $HeaderBitmap
FunctionEnd

Function un.SetupArtwork
  SendMessage $mui.Header.Image ${STM_GETIMAGE} ${IMAGE_BITMAP} 0 $HeaderBitmap
  !insertmacro TA_SMOOTH_BITMAP $mui.Header.Image "$PLUGINSDIR\modern-header.bmp" 600 228 $HeaderBitmap
FunctionEnd

Function .onGUIEnd
  SendMessage $mui.Header.Image ${STM_SETIMAGE} ${IMAGE_BITMAP} 0
  ${NSD_FreeImage} $HeaderBitmap
FunctionEnd

Function un.onGUIEnd
  SendMessage $mui.Header.Image ${STM_SETIMAGE} ${IMAGE_BITMAP} 0
  ${NSD_FreeImage} $HeaderBitmap
FunctionEnd

Function WelcomeArtwork
  !insertmacro TA_SMOOTH_BITMAP $mui.WelcomePage.Image "$PLUGINSDIR\modern-wizard.bmp" 656 1256 $mui.WelcomePage.Image.Bitmap
FunctionEnd

Function FinishArtwork
  !insertmacro TA_SMOOTH_BITMAP $mui.FinishPage.Image "$PLUGINSDIR\modern-wizard.bmp" 656 1256 $mui.FinishPage.Image.Bitmap
FunctionEnd

!macro TA_DEFINE_SHARED_FUNCTIONS UN

; Pushes the file name of a TA Reborn program (launcher or game) running from $INSTDIR, or
; "" if there is none. Programs of the same name elsewhere (e.g. a development build) are ignored.
Function ${UN}FindRunningApp
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  StrCpy $7 ""
  System::Call 'kernel32::CreateToolhelp32Snapshot(i 2, i 0) p .r0' ; TH32CS_SNAPPROCESS
  ${If} $0 P<> -1 ; INVALID_HANDLE_VALUE
    ; PROCESSENTRY32W as laid out for this 32-bit installer: dwSize, cntUsage, th32ProcessID,
    ; th32DefaultHeapID, th32ModuleID, cntThreads, th32ParentProcessID, pcPriClassBase, dwFlags,
    ; szExeFile[MAX_PATH]. &l4 fills in dwSize.
    System::Call '*(&l4, i, i, p, i, i, i, i, i, &w260) p .r1'
    System::Call 'kernel32::Process32FirstW(p r0, p r1) i .r2'
    ${DoWhile} $2 <> 0
      System::Call '*$1(i, i, i .r3, p, i, i, i, i, i, &w260 .r4)'
      ${If} $4 == "${LAUNCHER_EXE}"
      ${OrIf} $4 == "${GAME_EXE}"
        StrCpy $6 ""
        System::Call 'kernel32::OpenProcess(i 0x1000, i 0, i r3) p .r5' ; PROCESS_QUERY_LIMITED_INFORMATION
        ${If} $5 P<> 0
          System::Call 'kernel32::QueryFullProcessImageNameW(p r5, i 0, w .r6, *i ${NSIS_MAX_STRLEN}) i .r2'
          ${IfThen} $2 = 0 ${|} StrCpy $6 "" ${|}
          System::Call 'kernel32::CloseHandle(p r5)'
        ${EndIf}
        ; A program whose path cannot be read counts as ours.
        ${If} $6 == ""
        ${OrIf} $6 == "$INSTDIR\$4"
          StrCpy $7 $4
          ${ExitDo}
        ${EndIf}
      ${EndIf}
      System::Call 'kernel32::Process32NextW(p r0, p r1) i .r2'
    ${Loop}
    System::Free $1
    System::Call 'kernel32::CloseHandle(p r0)'
  ${EndIf}
  StrCpy $0 $7
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; Files of a running program can be neither replaced nor removed, so this asks the user to close
; TA Reborn while it runs from $INSTDIR. Pushes 1 when nothing (or nothing any more) is
; running, 0 when the user cancels - always the case in silent mode.
Function ${UN}EnsureNotRunning
  Push $0
  Push $1
  Push $2
  StrCpy $1 1
  StrCpy $2 0
  ${Do}
    Call ${UN}FindRunningApp
    Pop $0
    ${If} $0 == ""
      ${ExitDo}
    ${EndIf}
    ; (Updating, the launcher that started Setup is closing: it is given a moment first.)
    ${If} $Updating = 1
    ${AndIf} $0 == "${LAUNCHER_EXE}"
    ${AndIf} $2 < 40
      IntOp $2 $2 + 1
      Sleep 250
      ${Continue}
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APP_NAME} is running ($0).$\r$\n$\r$\nClose the launcher and the game, then click Retry." /SD IDCANCEL IDRETRY retry
    StrCpy $1 0
    ${ExitDo}
  retry:
  ${Loop}
  StrCpy $0 $1
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; Pushes the TA Reborn folder in the roaming AppData of the user running Setup - the folder
; the game and the launcher use: %APPDATA% from the environment, else the known folder - or ""
; if neither is available. Call it with SetShellVarContext current.
Function ${UN}GetUserDataDir
  Push $0
  ReadEnvStr $0 APPDATA
  ${If} $0 == ""
    StrCpy $0 $APPDATA
  ${EndIf}
  ${If} $0 != ""
    StrCpy $0 "$0\${APP_NAME}"
  ${EndIf}
  Exch $0
FunctionEnd

!macroend

!insertmacro TA_DEFINE_SHARED_FUNCTIONS ""
!insertmacro TA_DEFINE_SHARED_FUNCTIONS "un."

;--------------------------------------------------------------------------------------------
; Installer helpers

; Pops a folder path; pushes it without surrounding spaces or quotes and without a trailing
; backslash (except for a drive root such as D:\).
Function NormalizeDir
  Exch $0
  Push $1
  ${Do}
    StrCpy $1 $0 1
    ${If} $1 == " "
    ${OrIf} $1 == "$\t"
    ${OrIf} $1 == '"'
      StrCpy $0 $0 "" 1
    ${Else}
      ${ExitDo}
    ${EndIf}
  ${Loop}
  ${Do}
    StrCpy $1 $0 1 -1
    ${If} $1 == "\"
    ${OrIf} $1 == "/"
      StrLen $1 $0
      ${IfThen} $1 <= 3 ${|} ${ExitDo} ${|}
      StrCpy $0 $0 -1
    ${ElseIf} $1 == " "
    ${OrIf} $1 == "$\t"
    ${OrIf} $1 == '"'
      StrCpy $0 $0 -1
    ${Else}
      ${ExitDo}
    ${EndIf}
  ${Loop}
  ; "D:" alone would mean the current folder of drive D.
  StrLen $1 $0
  ${If} $1 = 2
    StrCpy $1 $0 1 1
    ${IfThen} $1 == ":" ${|} StrCpy $0 "$0\" ${|}
  ${EndIf}
  Pop $1
  Exch $0
FunctionEnd

; Pops a folder; pushes 1 if it contains the archives the launcher requires (totala1.hpi and
; totala2.hpi), else 0.
Function IsGameDir
  Exch $0
  Push $1
  StrCpy $1 $0 1 -1
  ${If} $1 != "\"
    StrCpy $0 "$0\"
  ${EndIf}
  ${If} $0 != "\"
  ${AndIf} ${FileExists} "$0totala1.hpi"
  ${AndIf} ${FileExists} "$0totala2.hpi"
    StrCpy $0 1
  ${Else}
    StrCpy $0 0
  ${EndIf}
  Pop $1
  Exch $0
FunctionEnd

; Used by FindInstall: if no install was found yet ($1 = ""), takes the candidate in $0.
!macro TA_CHECK_CANDIDATE
  ${If} $1 == ""
  ${AndIf} $0 != ""
    Push $0
    Call NormalizeDir
    Pop $0
    Push $0
    Call IsGameDir
    Pop $2
    ${IfThen} $2 == 1 ${|} StrCpy $1 $0 ${|}
  ${EndIf}
!macroend

; Looks for Total Annihilation like the launcher does: the GOG registry key (written by the GOG
; offline installer and by GOG Galaxy), the Steam release (its uninstall key, then Steam's own
; library), then the usual folders. Pushes the folder, or "" if none was found. (Steam libraries
; on other drives are the launcher's to find: it reads Steam's list of them.)
Function FindInstall
  Push $0
  Push $1
  Push $2
  StrCpy $1 ""
  ; HKLM32 is HKLM\SOFTWARE\WOW6432Node on 64-bit Windows, where GOG registers the game.
  ReadRegStr $0 HKLM32 "${GOG_GAME_KEY}" "path"
  !insertmacro TA_CHECK_CANDIDATE
  ReadRegStr $0 HKLM64 "${GOG_GAME_KEY}" "path"
  !insertmacro TA_CHECK_CANDIDATE
  ReadRegStr $0 HKCU "${GOG_GAME_KEY}" "path"
  !insertmacro TA_CHECK_CANDIDATE
  ReadRegStr $0 HKLM32 "${STEAM_GAME_KEY}" "InstallLocation"
  !insertmacro TA_CHECK_CANDIDATE
  ReadRegStr $0 HKLM64 "${STEAM_GAME_KEY}" "InstallLocation"
  !insertmacro TA_CHECK_CANDIDATE
  ReadRegStr $0 HKCU "Software\Valve\Steam" "SteamPath"
  ${If} $0 != ""
    StrCpy $0 "$0\steamapps\common\Total Annihilation"
    !insertmacro TA_CHECK_CANDIDATE
  ${EndIf}
  ReadRegStr $0 HKLM32 "${STEAM_KEY}" "InstallPath"
  ${If} $0 != ""
    StrCpy $0 "$0\steamapps\common\Total Annihilation"
    !insertmacro TA_CHECK_CANDIDATE
  ${EndIf}
  StrCpy $0 "$PROGRAMFILES32\Steam\steamapps\common\Total Annihilation"
  !insertmacro TA_CHECK_CANDIDATE
  Call GetSystemDrive
  Pop $0
  StrCpy $0 "$0\GOG Games\Total Annihilation"
  !insertmacro TA_CHECK_CANDIDATE
  StrCpy $0 "$PROGRAMFILES32\GOG Galaxy\Games\Total Annihilation"
  !insertmacro TA_CHECK_CANDIDATE
  StrCpy $0 "$PROGRAMFILES32\GOG.com\Total Annihilation"
  !insertmacro TA_CHECK_CANDIDATE
!ifdef TEST_PER_USER
  ${IfThen} $NoDetect == 1 ${|} StrCpy $1 "" ${|}
!endif
  ClearErrors
  StrCpy $0 $1
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; Pushes the drive Windows is installed on, e.g. "C:".
Function GetSystemDrive
  Push $0
  ReadEnvStr $0 SystemDrive
  ${If} $0 == ""
    StrCpy $0 $WINDIR 2
  ${EndIf}
  Exch $0
FunctionEnd

; Pops a path; pushes it as a TOML string. That is a literal string ('...', in which backslashes
; need no escaping) unless the path contains a single quote; then it is a basic string ("...")
; with \\ and \" escapes. config.toml is written as UTF-8, so nothing else needs escaping.
Function TomlString
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  StrCpy $1 ""
  StrCpy $2 0
  StrCpy $3 0
  ${Do}
    StrCpy $4 $0 1 $2
    ${If} $4 == ""
      ${ExitDo}
    ${ElseIf} $4 == "'"
      StrCpy $3 1
    ${ElseIf} $4 == "\"
      StrCpy $4 "\\"
    ${ElseIf} $4 == '"'
      StrCpy $4 '\"'
    ${EndIf}
    StrCpy $1 "$1$4"
    IntOp $2 $2 + 1
  ${Loop}
  ${If} $3 = 0
    StrCpy $0 "'$0'"
  ${Else}
    StrCpy $0 '"$1"'
  ${EndIf}
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; Pops a text, then a file path; writes the text to the file as UTF-8 without BOM (FileWrite
; would convert it to the ANSI code page). Pushes 1 on success, else 0.
Function WriteUtf8File
  Exch $1
  Exch
  Exch $0
  Push $2
  Push $3
  Push $4
  Push $5
  StrCpy $5 0
  ; Buffer size in bytes, including the terminating NUL.
  System::Call 'kernel32::WideCharToMultiByte(i 65001, i 0, w r1, i -1, p 0, i 0, p 0, p 0) i .r2'
  ${If} $2 > 0
    System::Alloc $2
    Pop $3
    System::Call 'kernel32::WideCharToMultiByte(i 65001, i 0, w r1, i -1, p r3, i r2, p 0, p 0) i .r2'
    IntOp $2 $2 - 1
    ClearErrors
    FileOpen $4 "$0" w
    ${IfNot} ${Errors}
      System::Call 'kernel32::WriteFile(p r4, p r3, i r2, *i .r0, p 0) i .r5'
      ${If} $5 <> 0
      ${AndIf} $0 = $2
        StrCpy $5 1
      ${Else}
        StrCpy $5 0
      ${EndIf}
      FileClose $4
    ${EndIf}
    System::Free $3
  ${EndIf}
  StrCpy $0 $5
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; Records the game folder for the launcher in %APPDATA%\TA Reborn\config.toml of the user
; running Setup - only if that user has no settings file yet, so reinstalls and upgrades never
; overwrite their settings, and only for a valid game folder.
Function WriteLauncherConfig
  ${If} $GameDir == ""
    DetailPrint "The original game is not set up yet: the launcher will help to find or get it."
    Return
  ${EndIf}
  Push $GameDir
  Call IsGameDir
  Pop $0
  ${If} $0 != 1
    DetailPrint "Not recording $GameDir: it does not contain totala1.hpi and totala2.hpi."
    Return
  ${EndIf}
  SetShellVarContext current
  Call GetUserDataDir
  Pop $1
  ; Only the folder under the game's old name so far: the settings go in there, and move with
  ; it when the game first starts.
  ${If} $1 != ""
  ${AndIfNot} ${FileExists} "$1\*.*"
    ${GetParent} $1 $3
    ${If} ${FileExists} "$3\${OLD_APP_NAME}\*.*"
      StrCpy $1 "$3\${OLD_APP_NAME}"
    ${EndIf}
  ${EndIf}
  ${If} $1 == ""
    DetailPrint "Could not find the Application Data folder; the launcher will look for the game itself."
  ${ElseIf} ${FileExists} "$1\config.toml"
    DetailPrint "Keeping the existing settings file $1\config.toml"
  ${Else}
    CreateDirectory "$1"
    Push $GameDir
    Call TomlString
    Pop $2
    Push "$1\config.toml"
    Push "# Written by the TA Reborn installer. Other settings are managed by the launcher.$\r$\ngame_dir = $2$\r$\n"
    Call WriteUtf8File
    Pop $0
    ${If} $0 == 1
      DetailPrint "Recorded the original game ($GameDir) in $1\config.toml"
    ${Else}
      DetailPrint "Could not write $1\config.toml; the launcher will look for the game itself."
    ${EndIf}
  ${EndIf}
  SetShellVarContext ${SHELL_CONTEXT}
FunctionEnd

; Pops a program or URL and opens it as the signed-in user. Setup runs elevated, but the
; launcher must not (it writes the user's settings), and neither should the web browser:
; Explorer, which runs as the signed-in user, opens it instead.
Function OpenAsUser
  Exch $0
  ClearErrors
  Exec '"$WINDIR\explorer.exe" "$0"'
  ${If} ${Errors}
    ClearErrors
    ExecShell "open" "$0"
  ${EndIf}
  Pop $0
FunctionEnd

Function LaunchLauncher
  Push "$INSTDIR\${LAUNCHER_EXE}"
  Call OpenAsUser
FunctionEnd

; The pages an update has no need of (it goes over the installed version, as it was set up).
Function SkipWhenUpdating
  ${If} $Updating = 1
    Abort
  ${EndIf}
FunctionEnd

; An update is done: the launcher starts again, on the new version.
Function .onInstSuccess
  ${If} $Updating = 1
    Call LaunchLauncher
  ${EndIf}
FunctionEnd

; Removes the installation under the game's old name with its own uninstaller, silently (so it
; keeps the user's settings, saved games and mods), and brings mods put in its program folder
; to this one's.
Function RemoveOldVersion
  DetailPrint "Removing ${OLD_APP_NAME}, this game's earlier name, from $OldInstDir"
  ; (_?= runs the uninstaller in place and waits for it; it cannot delete itself then.)
  ClearErrors
  ExecWait '"$OldInstDir\${UNINSTALL_EXE}" /S _?=$OldInstDir' $0
  ${If} ${Errors}
  ${OrIf} $0 <> 0
    DetailPrint "${OLD_APP_NAME} was not removed (is it running?): remove it in Settings, Apps."
    MessageBox MB_OK|MB_ICONEXCLAMATION "${OLD_APP_NAME}, this game's earlier name, could not be removed (is it running?).$\r$\n$\r$\nRemove it in Settings, Apps, once Setup has finished." /SD IDOK
    Return
  ${EndIf}
  Delete "$OldInstDir\${UNINSTALL_EXE}"
  ${If} $OldInstDir != $INSTDIR
  ${AndIf} ${FileExists} "$OldInstDir\mods\*.*"
  ${AndIfNot} ${FileExists} "$INSTDIR\mods\*.*"
    CreateDirectory "$INSTDIR"
    ClearErrors
    Rename "$OldInstDir\mods" "$INSTDIR\mods"
    ${If} ${Errors}
      DetailPrint "Mods were left in $OldInstDir\mods: move them to $INSTDIR\mods."
    ${Else}
      DetailPrint "Moved the mods in $OldInstDir\mods to $INSTDIR\mods"
    ${EndIf}
  ${EndIf}
  RMDir "$OldInstDir"
FunctionEnd

;--------------------------------------------------------------------------------------------
; Directory page

Function DirectoryPagePre
  ; An upgrade installs over the previous version, in its folder.
  ${If} $PrevInstDir != ""
  ${OrIf} $Updating = 1
    Abort
  ${EndIf}
FunctionEnd

; The uninstaller removes the assets and docs folders: keep TA Reborn out of the original
; game's folder (which has a Docs folder of its own).
!macro TA_REFUSE_GAME_FOLDER
  MessageBox MB_OK|MB_ICONEXCLAMATION "$INSTDIR$\r$\ncontains the original Total Annihilation.$\r$\n$\r$\nInstall ${APP_NAME} into a folder of its own, for example $PROGRAMFILES64\${APP_NAME}. It finds the original game wherever it is installed." /SD IDOK
!macroend

Function DirectoryPageLeave
  ${If} ${FileExists} "$INSTDIR\totala1.hpi"
    !insertmacro TA_REFUSE_GAME_FOLDER
    Abort
  ${EndIf}
FunctionEnd

;--------------------------------------------------------------------------------------------
; "Original game" page: confirms the detected install, or explains what is needed.

Function GamePageCreate
  ; (An update keeps the game as it was set up.)
  ${If} $Updating = 1
    Abort
  ${EndIf}
  ; Look again every time the page is shown: the user may have just installed the game.
  Call FindInstall
  Pop $1
  StrCpy $GameFound 0
  StrCpy $0 ""
  ${If} $GameChoice == "use"
  ${AndIf} $GameDirText != ""
    ; A folder the user picked earlier wins over the detected one.
    Push $GameDirText
    Call NormalizeDir
    Pop $2
    Push $2
    Call IsGameDir
    Pop $3
    ${If} $3 == 1
      StrCpy $0 $2
      StrCpy $GameFound 1
    ${EndIf}
  ${EndIf}
  ${If} $GameFound = 0
  ${AndIf} $1 != ""
    StrCpy $0 $1
    StrCpy $GameFound 1
  ${EndIf}

  ${If} $GameFound = 1
    !insertmacro MUI_HEADER_TEXT "Original game" "Setup found your copy of Total Annihilation."
  ${Else}
    !insertmacro MUI_HEADER_TEXT "Original game required" "${APP_NAME} needs your own copy of Total Annihilation."
  ${EndIf}

  nsDialogs::Create 1018
  Pop $2
  ${If} $2 == error
    Abort
  ${EndIf}

  ${If} $GameFound = 1
    ${NSD_CreateLabel} 0 0 100% 28u "${APP_NAME} plays the data files of your copy of Total Annihilation, read in place from its folder: nothing there is copied or changed. Setup found the game here:"
    Pop $2
    ${NSD_CreateDirRequest} 0 32u 236u 12u "$0"
    Pop $GameDirField
    ${NSD_CreateBrowseButton} 242u 31u 58u 14u "$(^BrowseBtn)"
    Pop $GameBrowseButton
    ${NSD_CreateLabel} 0 50u 100% 10u ""
    Pop $GameStatusLabel
    ${NSD_CreateLabel} 0 70u 100% 20u "To use a different installation, click Browse and select the folder that contains totala1.hpi. You can also change it later on the launcher's Play page."
    Pop $2
  ${Else}
    ${NSD_CreateLabel} 0 0 100% 34u "Setup could not find Total Annihilation on this PC. ${APP_NAME} contains none of the original game's units, maps, artwork, sound or music: it needs a legitimately owned copy, such as $\"Total Annihilation: Commander Pack$\" from GOG.com or Steam (which include both expansions)."
    Pop $2
    ${NSD_CreateButton} 0 37u 92u 14u "&Get it on GOG.com"
    Pop $2
    ${NSD_OnClick} $2 GameStoreClick
    ${NSD_CreateButton} 98u 37u 80u 14u "Get it on &Steam"
    Pop $2
    ${NSD_OnClick} $2 GameSteamClick
    ${NSD_CreateButton} 184u 37u 70u 14u "&Detect again"
    Pop $2
    ${NSD_OnClick} $2 GameDetectClick
    ${NSD_CreateRadioButton} 0 57u 100% 10u "&Use my existing installation in this folder:"
    Pop $GameUseRadio
    ${NSD_OnClick} $GameUseRadio GameChoiceClick
    ${NSD_CreateDirRequest} 12u 69u 224u 12u "$GameDirText"
    Pop $GameDirField
    ${NSD_CreateBrowseButton} 242u 68u 58u 14u "$(^BrowseBtn)"
    Pop $GameBrowseButton
    ${NSD_CreateLabel} 12u 84u 288u 10u ""
    Pop $GameStatusLabel
    ${NSD_CreateRadioButton} 0 98u 100% 10u "&Continue without it and set it up later in the launcher"
    Pop $GameLaterRadio
    ${NSD_OnClick} $GameLaterRadio GameChoiceClick
    ${NSD_CreateLabel} 12u 110u 288u 28u "Warning: ${APP_NAME} cannot be played until the original game is installed. Once it is, start ${APP_NAME} and select the game's folder on the launcher's Play page."
    Pop $2
    ${If} $BoldFont == ""
      CreateFont $BoldFont "$(^Font)" "$(^FontSize)" "700"
    ${EndIf}
    SendMessage $2 ${WM_SETFONT} $BoldFont 0
    ${If} $GameChoice == "use"
      ${NSD_Check} $GameUseRadio
    ${Else}
      ${NSD_Check} $GameLaterRadio
    ${EndIf}
  ${EndIf}
  ${NSD_OnClick} $GameBrowseButton GameBrowseClick
  ${NSD_OnChange} $GameDirField GameDirChange
  Call GameUpdatePage
  nsDialogs::Show
FunctionEnd

; Enables the folder controls for the chosen option and says whether the folder is usable.
Function GameUpdatePage
  ${NSD_GetText} $GameDirField $GameDirText
  ${If} $GameFound = 1
    StrCpy $GameChoice "use"
  ${Else}
    ${NSD_GetState} $GameUseRadio $0
    ${If} $0 == ${BST_CHECKED}
      StrCpy $GameChoice "use"
    ${Else}
      StrCpy $GameChoice "later"
    ${EndIf}
    ${If} $GameChoice == "use"
      EnableWindow $GameDirField 1
      EnableWindow $GameBrowseButton 1
    ${Else}
      EnableWindow $GameDirField 0
      EnableWindow $GameBrowseButton 0
    ${EndIf}
  ${EndIf}
  Push $GameDirText
  Call NormalizeDir
  Pop $0
  StrCpy $2 0 ; 0 = neutral, 1 = found, 2 = not a game folder
  ${If} $GameFound = 0
  ${AndIf} $GameChoice != "use"
    StrCpy $1 ""
  ${ElseIf} $0 == ""
    StrCpy $1 "Choose the folder that contains totala1.hpi and totala2.hpi."
  ${Else}
    Push $0
    Call IsGameDir
    Pop $3
    ${If} $3 == 1
      StrCpy $1 "Total Annihilation found in this folder."
      StrCpy $2 1
    ${Else}
      StrCpy $1 "This folder does not contain totala1.hpi and totala2.hpi."
      StrCpy $2 2
    ${EndIf}
  ${EndIf}
  ${If} $2 = 1
    SetCtlColors $GameStatusLabel 0x007000 transparent
  ${ElseIf} $2 = 2
    SetCtlColors $GameStatusLabel 0xC00000 transparent
  ${Else}
    SetCtlColors $GameStatusLabel SYSCLR:18 transparent ; COLOR_BTNTEXT
  ${EndIf}
  ${NSD_SetText} $GameStatusLabel $1
  ; A transparent label does not erase its old text: hide and show it to repaint the background.
  ShowWindow $GameStatusLabel ${SW_HIDE}
  ShowWindow $GameStatusLabel ${SW_SHOW}
FunctionEnd

Function GameChoiceClick
  Pop $0
  ; Keep the two options exclusive even though other controls sit between them.
  ${If} $0 == $GameUseRadio
    ${NSD_Uncheck} $GameLaterRadio
  ${Else}
    ${NSD_Uncheck} $GameUseRadio
  ${EndIf}
  Call GameUpdatePage
FunctionEnd

Function GameDirChange
  Pop $0
  Call GameUpdatePage
FunctionEnd

; Puts a valid game folder into the field and selects "use my existing installation".
Function GameUseFolder
  Exch $0
  ${If} $GameFound = 0
    ${NSD_Check} $GameUseRadio
    ${NSD_Uncheck} $GameLaterRadio
  ${EndIf}
  ${NSD_SetText} $GameDirField $0
  Call GameUpdatePage
  Pop $0
FunctionEnd

Function GameBrowseClick
  Pop $0
  ${NSD_GetText} $GameDirField $0
  Push $0
  Call NormalizeDir
  Pop $0
  ${If} $0 == ""
  ${OrIfNot} ${FileExists} "$0\*.*"
    Call GetSystemDrive
    Pop $0
    StrCpy $0 "$0\GOG Games"
    ${IfNotThen} ${FileExists} "$0\*.*" ${|} StrCpy $0 "" ${|}
  ${EndIf}
  nsDialogs::SelectFolderDialog "Select the folder where Total Annihilation is installed (the one that contains totala1.hpi):" "$0"
  Pop $0
  ${If} $0 == error
    Return
  ${EndIf}
  Push $0
  Call NormalizeDir
  Pop $0
  Push $0
  Call IsGameDir
  Pop $1
  ${If} $1 != 1
    ; People often pick the parent folder, such as C:\GOG Games.
    StrCpy $2 $0 1 -1
    ${If} $2 == "\"
      StrCpy $2 "$0Total Annihilation"
    ${Else}
      StrCpy $2 "$0\Total Annihilation"
    ${EndIf}
    Push $2
    Call IsGameDir
    Pop $1
    ${IfThen} $1 == 1 ${|} StrCpy $0 $2 ${|}
  ${EndIf}
  ${If} $1 == 1
    Push $0
    Call GameUseFolder
  ${Else}
    MessageBox MB_OK|MB_ICONEXCLAMATION "$0$\r$\ndoes not look like a Total Annihilation installation: totala1.hpi and totala2.hpi are not in this folder.$\r$\n$\r$\nSelect the folder where Total Annihilation is installed, for example C:\GOG Games\Total Annihilation, or Total Annihilation in Steam's steamapps\common folder."
  ${EndIf}
FunctionEnd

Function GameStoreClick
  Pop $0
  Push "${GOG_STORE_URL}"
  Call OpenAsUser
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONEXCLAMATION "Your web browser could not be opened. Please visit:$\r$\n${GOG_STORE_URL}"
  ${EndIf}
FunctionEnd

Function GameSteamClick
  Pop $0
  Push "${STEAM_STORE_URL}"
  Call OpenAsUser
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONEXCLAMATION "Your web browser could not be opened. Please visit:$\r$\n${STEAM_STORE_URL}"
  ${EndIf}
FunctionEnd

Function GameDetectClick
  Pop $0
  Call FindInstall
  Pop $0
  ${If} $0 != ""
    Push $0
    Call GameUseFolder
    MessageBox MB_OK|MB_ICONINFORMATION "Total Annihilation was found in:$\r$\n$0"
  ${Else}
    MessageBox MB_OK|MB_ICONINFORMATION "Total Annihilation still could not be found. If you have just bought it, install it with GOG Galaxy, the GOG offline installer or Steam, then click $\"Detect again$\". (A Steam library on another drive: click Browse and select the game's folder.)"
  ${EndIf}
FunctionEnd

Function GamePageLeave
  ${NSD_GetText} $GameDirField $0
  Push $0
  Call NormalizeDir
  Pop $0
  ${If} $GameFound = 1
  ${OrIf} $GameChoice == "use"
    Push $0
    Call IsGameDir
    Pop $1
    ${If} $1 != 1
      ${If} $GameFound = 1
        MessageBox MB_OK|MB_ICONEXCLAMATION "The selected folder does not contain totala1.hpi and totala2.hpi.$\r$\n$\r$\nChoose the folder where Total Annihilation is installed, for example C:\GOG Games\Total Annihilation."
      ${Else}
        MessageBox MB_OK|MB_ICONEXCLAMATION "The selected folder does not contain totala1.hpi and totala2.hpi.$\r$\n$\r$\nChoose the folder where Total Annihilation is installed (for example C:\GOG Games\Total Annihilation), or select $\"Continue without it and set it up later in the launcher$\"."
      ${EndIf}
      Abort
    ${EndIf}
    StrCpy $GameDir $0
  ${Else}
    StrCpy $GameDir ""
  ${EndIf}
  ; This is the last page before files are copied.
  Call EnsureNotRunning
  Pop $1
  ${If} $1 != 1
    Abort
  ${EndIf}
FunctionEnd

;--------------------------------------------------------------------------------------------
; File types that open in the game (double-click, or Open with): the extension, its ProgID and
; the name Explorer shows. Uninstalling removes them again, the extension's default only if it
; still names our ProgID.

!macro TA_ASSOCIATE EXT PROGID DESCRIPTION
  WriteRegStr SHCTX "Software\Classes\${EXT}" "" "${PROGID}"
  WriteRegStr SHCTX "Software\Classes\${EXT}\OpenWithProgids" "${PROGID}" ""
  WriteRegStr SHCTX "Software\Classes\${PROGID}" "" "${DESCRIPTION}"
  WriteRegStr SHCTX "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\${GAME_EXE},0"
  WriteRegStr SHCTX "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\${GAME_EXE}" "%1"'
!macroend

!macro TA_UNASSOCIATE EXT PROGID
  ReadRegStr $0 SHCTX "Software\Classes\${EXT}" ""
  ${If} $0 == "${PROGID}"
    DeleteRegValue SHCTX "Software\Classes\${EXT}" ""
  ${EndIf}
  DeleteRegValue SHCTX "Software\Classes\${EXT}\OpenWithProgids" "${PROGID}"
  DeleteRegKey /ifempty SHCTX "Software\Classes\${EXT}\OpenWithProgids"
  DeleteRegKey /ifempty SHCTX "Software\Classes\${EXT}"
  DeleteRegKey SHCTX "Software\Classes\${PROGID}"
!macroend

;--------------------------------------------------------------------------------------------
; Sections

Section "${APP_NAME} (required)" SecProgram
  SectionIn RO
  SetShellVarContext ${SHELL_CONTEXT}
  SetRegView 64

  ; (An update shows its progress, then goes on by itself.)
  ${If} $Updating = 1
    SetAutoClose true
  ${EndIf}
  ; The pages check these too, but silent installs skip the pages.
  ${If} ${FileExists} "$INSTDIR\totala1.hpi"
    !insertmacro TA_REFUSE_GAME_FOLDER
    DetailPrint "Not installing into the original game's folder $INSTDIR."
    Abort
  ${EndIf}
  Call EnsureNotRunning
  Pop $0
  ${If} $0 != 1
    DetailPrint "${APP_NAME} is running from $INSTDIR: close it and run Setup again."
    Abort
  ${EndIf}
  ${If} $OldInstDir != ""
    Call RemoveOldVersion
  ${EndIf}

  SetOutPath "$INSTDIR"
  ; Upgrades: drop the previous version's engine resources and documentation, so that no stale
  ; files linger. Everything else is simply overwritten.
  ${If} ${FileExists} "$INSTDIR\${LAUNCHER_EXE}"
  ${AndIf} ${FileExists} "$INSTDIR\${UNINSTALL_EXE}"
    RMDir /r "$INSTDIR\assets"
    RMDir /r "$INSTDIR\docs"
    RMDir /r "$INSTDIR\lang"
    ; Up to 1.12 the licences were LICENSE-MIT and LICENSE-APACHE; it is LICENSE.txt now.
    Delete "$INSTDIR\LICENSE-MIT"
    Delete "$INSTDIR\LICENSE-APACHE"
  ${EndIf}

  ; Everything that was staged. The exclusions are a second guard (build_installer.ps1 is the
  ; first): TA Reborn never redistributes the original game's archives.
  File /r /x *.hpi /x *.ufo /x *.ccx /x *.gp3 /x *.gpf "${STAGE_DIR}\*"
  CreateDirectory "$INSTDIR\assets"
  CreateDirectory "$INSTDIR\mods"

  WriteUninstaller "$INSTDIR\${UNINSTALL_EXE}"

  ; Shortcuts start in $OUTDIR, the program folder.
  CreateDirectory "${SM_DIR}"
  CreateShortcut "${SM_DIR}\${APP_NAME}.lnk" "$INSTDIR\${LAUNCHER_EXE}" "" "" 0 SW_SHOWNORMAL "" "Launch ${APP_NAME}"
  CreateShortcut "${SM_DIR}\${APP_NAME} (direct).lnk" "$INSTDIR\${GAME_EXE}" "" "" 0 SW_SHOWNORMAL "" "Start the game without the launcher"
  CreateShortcut "${SM_DIR}\Uninstall ${APP_NAME}.lnk" "$INSTDIR\${UNINSTALL_EXE}" "" "" 0 SW_SHOWNORMAL "" "Uninstall ${APP_NAME}"
SectionEnd

Section /o "Desktop shortcut" SecDesktop
  SetShellVarContext ${SHELL_CONTEXT}
  SetOutPath "$INSTDIR"
  CreateShortcut "${DESKTOP_LNK}" "$INSTDIR\${LAUNCHER_EXE}" "" "" 0 SW_SHOWNORMAL "" "Launch ${APP_NAME}"
SectionEnd

Section "-Register"
  SetShellVarContext ${SHELL_CONTEXT}
  SetRegView 64
  ${IfNot} ${SectionIsSelected} ${SecDesktop}
    Delete "${DESKTOP_LNK}"
  ${EndIf}

  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "DisplayName" "${DISPLAY_NAME}"
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "Comments" "${APP_COMMENTS}"
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${LAUNCHER_EXE}"
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "UninstallString" '"$INSTDIR\${UNINSTALL_EXE}"'
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\${UNINSTALL_EXE}" /S'
  WriteRegDWORD ${UNINST_ROOT} "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD ${UNINST_ROOT} "${UNINST_KEY}" "NoRepair" 1
!ifdef APP_URL
  WriteRegStr ${UNINST_ROOT} "${UNINST_KEY}" "URLInfoAbout" "${APP_URL}"
!endif
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD ${UNINST_ROOT} "${UNINST_KEY}" "EstimatedSize" $0

  !insertmacro TA_ASSOCIATE ".tarep" "TAReborn.Replay" "${APP_NAME} replay"
  !insertmacro TA_ASSOCIATE ".tasav" "TAReborn.SavedGame" "${APP_NAME} saved game"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)' ; SHCNE_ASSOCCHANGED

  Call WriteLauncherConfig

  ${If} $GameDir == ""
    StrCpy $FinishText "${APP_NAME} has been installed on your computer.$\r$\n$\r$\nBefore you can play, install Total Annihilation (for example $\"Total Annihilation: Commander Pack$\" from GOG.com), then select its folder on the launcher's Play page.$\r$\n$\r$\nClick Finish to close Setup."
  ${Else}
    StrCpy $FinishText "${APP_NAME} has been installed on your computer.$\r$\n$\r$\nClick Finish to close Setup."
  ${EndIf}
SectionEnd

;--------------------------------------------------------------------------------------------
; Installer initialisation

Function .onInit
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_OK|MB_ICONSTOP "${APP_NAME} requires Windows 10 or later." /SD IDOK
    SetErrorLevel 1633 ; ERROR_INSTALL_PLATFORM_UNSUPPORTED
    Abort
  ${EndIf}
!if "${APP_ARCH}" == "arm64"
  ${IfNot} ${IsNativeARM64}
  ${OrIfNot} ${AtLeastWin11}
    MessageBox MB_OK|MB_ICONSTOP "This ARM64 package requires Windows 11 on an ARM64 processor. Use the x64 package for an Intel or AMD computer." /SD IDOK
    SetErrorLevel 1633
    Abort
  ${EndIf}
!else
  ${IfNot} ${IsNativeAMD64}
    ; Existing x64 installations on Windows 11 ARM may update in place using the
    ; operating system's x64 emulation. Windows 10 ARM does not support this payload.
    ${IfNot} ${IsNativeARM64}
    ${OrIfNot} ${AtLeastWin11}
      MessageBox MB_OK|MB_ICONSTOP "This x64 package requires 64-bit Intel/AMD Windows 10 or later, or Windows 11 ARM64 with x64 emulation." /SD IDOK
      SetErrorLevel 1633
      Abort
    ${EndIf}
  ${EndIf}
!endif
  SetRegView 64
  SetShellVarContext ${SHELL_CONTEXT}
!ifndef TEST_PER_USER
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 != "admin"
    MessageBox MB_OK|MB_ICONSTOP "Administrator rights are required to install ${APP_NAME}." /SD IDOK
    SetErrorLevel 740 ; ERROR_ELEVATION_REQUIRED
    Quit
  ${EndIf}
!endif

  ; Upgrades go into the folder of the installed version, unless /D= names another one (/D= sets
  ; $INSTDIR before .onInit; without it $INSTDIR still holds the default here).
  ReadRegStr $PrevInstDir ${UNINST_ROOT} "${UNINST_KEY}" "InstallLocation"
  ${If} $PrevInstDir != ""
  ${AndIf} ${FileExists} "$PrevInstDir\*.*"
    ${If} $INSTDIR == "${DEFAULT_INSTDIR}"
      StrCpy $INSTDIR $PrevInstDir
    ${EndIf}
  ${Else}
    StrCpy $PrevInstDir ""
  ${EndIf}
  ; An installation under the game's old name: this version goes beside it under the new name
  ; (or into it, if its folder has a name of its own), unless this version is installed already
  ; or /D= names a folder.
  ReadRegStr $OldInstDir ${UNINST_ROOT} "${OLD_UNINST_KEY}" "InstallLocation"
  ${If} $OldInstDir != ""
  ${AndIf} ${FileExists} "$OldInstDir\${UNINSTALL_EXE}"
    ${If} $PrevInstDir == ""
    ${AndIf} $INSTDIR == "${DEFAULT_INSTDIR}"
      ${GetFileName} $OldInstDir $1
      ${If} $1 == "${OLD_APP_NAME}"
        ${GetParent} $OldInstDir $1
        StrCpy $INSTDIR "$1\${APP_NAME}"
      ${Else}
        StrCpy $INSTDIR $OldInstDir
      ${EndIf}
    ${EndIf}
  ${Else}
    StrCpy $OldInstDir ""
  ${EndIf}
!ifdef TEST_PER_USER
  ${If} $INSTDIR == ""
    MessageBox MB_OK|MB_ICONSTOP "This per-user test build needs a folder: /D=<folder> (last on the command line)." /SD IDOK
    Abort
  ${EndIf}
!endif

  ; The desktop shortcut stays as it was on upgrades (the old name's too); /DESKTOPSHORTCUT adds
  ; it in silent installs.
  ${If} ${FileExists} "${DESKTOP_LNK}"
  ${OrIf} ${FileExists} "$DESKTOP\${OLD_APP_NAME}.lnk"
    !insertmacro SelectSection ${SecDesktop}
  ${EndIf}
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/DESKTOPSHORTCUT" $1
  ${IfNot} ${Errors}
    !insertmacro SelectSection ${SecDesktop}
  ${EndIf}
  ; /UPDATE: the launcher updating itself, over a version Setup installed there.
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  ${IfNot} ${Errors}
  ${AndIf} ${FileExists} "$INSTDIR\${LAUNCHER_EXE}"
  ${AndIf} ${FileExists} "$INSTDIR\${UNINSTALL_EXE}"
    StrCpy $Updating 1
  ${EndIf}

!ifdef TEST_PER_USER
  ClearErrors
  ${GetOptions} $0 "/NODETECT" $1
  ${IfNot} ${Errors}
    StrCpy $NoDetect 1
  ${EndIf}
!endif

  ; Silent installs record the detected game as is (or the valid folder given with
  ; /GAMEDIR=<folder>); the "Original game" page lets users change it.
  StrCpy $GameChoice "later"
  Call FindInstall
  Pop $GameDir
  ClearErrors
  ${GetOptions} $0 "/GAMEDIR=" $1
  ${IfNot} ${Errors}
    Push $1
    Call NormalizeDir
    Pop $1
    Push $1
    Call IsGameDir
    Pop $2
    ${If} $2 == 1
      StrCpy $GameDir $1
      StrCpy $GameDirText $1
      StrCpy $GameChoice "use"
    ${EndIf}
  ${EndIf}
FunctionEnd

;--------------------------------------------------------------------------------------------
; Uninstaller

Function un.onInit
  SetRegView 64
  SetShellVarContext ${SHELL_CONTEXT}
!ifndef TEST_PER_USER
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 != "admin"
    MessageBox MB_OK|MB_ICONSTOP "Administrator rights are required to uninstall ${APP_NAME}." /SD IDOK
    SetErrorLevel 740 ; ERROR_ELEVATION_REQUIRED
    Quit
  ${EndIf}
!endif
  Call un.EnsureNotRunning
  Pop $0
  ${If} $0 != 1
    Abort
  ${EndIf}
FunctionEnd

Section "Uninstall"
  SetRegView 64
  SetShellVarContext ${SHELL_CONTEXT}

  ; Whole folders are only removed from what really is a TA Reborn installation, and never
  ; from the original game's folder.
  StrCpy $1 0
  ${If} ${FileExists} "$INSTDIR\${LAUNCHER_EXE}"
  ${OrIf} ${FileExists} "$INSTDIR\${GAME_EXE}"
    ${IfNot} ${FileExists} "$INSTDIR\totala1.hpi"
      StrCpy $1 1
    ${EndIf}
  ${EndIf}
  Delete "$INSTDIR\${LAUNCHER_EXE}"
  Delete "$INSTDIR\${GAME_EXE}"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\MICROSOFT-RUNTIME-NOTICE.txt"
  Delete "$INSTDIR\DXC-NOTICE.txt"
  ; Microsoft's DirectX Shader Compiler (windows_package.ps1's pinned release).
  !insertmacro DeleteBundledRuntime dxcompiler.dll
  !insertmacro DeleteBundledRuntime vcruntime140.dll
  !insertmacro DeleteBundledRuntime vcruntime140_1.dll
  !insertmacro DeleteBundledRuntime vcruntime140_threads.dll
  !insertmacro DeleteBundledRuntime msvcp140.dll
  !insertmacro DeleteBundledRuntime msvcp140_1.dll
  !insertmacro DeleteBundledRuntime msvcp140_2.dll
  !insertmacro DeleteBundledRuntime msvcp140_atomic_wait.dll
  !insertmacro DeleteBundledRuntime msvcp140_codecvt_ids.dll
  !insertmacro DeleteBundledRuntime concrt140.dll
  !insertmacro DeleteBundledRuntime vccorlib140.dll
  ${If} $1 = 1
    RMDir /r "$INSTDIR\assets"
    RMDir /r "$INSTDIR\docs"
    RMDir /r "$INSTDIR\lang"
  ${EndIf}
  ; Mods added to the program's mods folder are kept.
  Delete "$INSTDIR\mods\README.txt"
  RMDir "$INSTDIR\mods"
  Delete "$INSTDIR\${UNINSTALL_EXE}"
  RMDir "$INSTDIR"
  ${If} ${FileExists} "$INSTDIR\*.*"
    DetailPrint "Kept $INSTDIR: it contains files that Setup did not install (such as mods)."
  ${EndIf}

  Delete "${SM_DIR}\${APP_NAME}.lnk"
  Delete "${SM_DIR}\${APP_NAME} (direct).lnk"
  Delete "${SM_DIR}\Uninstall ${APP_NAME}.lnk"
  RMDir "${SM_DIR}"
  Delete "${DESKTOP_LNK}"

  DeleteRegKey ${UNINST_ROOT} "${UNINST_KEY}"
  !insertmacro TA_UNASSOCIATE ".tarep" "TAReborn.Replay"
  !insertmacro TA_UNASSOCIATE ".tasav" "TAReborn.SavedGame"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)' ; SHCNE_ASSOCCHANGED

  ; The current user's settings, campaign progress, saved games, replays, personal mods and logs
  ; stay unless the user asks otherwise (never in silent mode).
  SetShellVarContext current
  Call un.GetUserDataDir
  Pop $0
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\*.*"
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "Do you also want to delete your ${APP_NAME} settings, campaign progress, saved games, replays, personal mods and logs?$\r$\n$\r$\n$0$\r$\n$\r$\nChoose No to keep them, for example to reinstall ${APP_NAME} later." /SD IDNO IDNO keep_user_data
    RMDir /r "$0"
    DetailPrint "Deleted $0"
  keep_user_data:
  ${EndIf}
  SetShellVarContext ${SHELL_CONTEXT}
SectionEnd
