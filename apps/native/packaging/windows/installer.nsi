Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!define PRODUCT "Workspacer Native"
!define APP_ID "Workspacer.Native"
; The identity this product shipped under while it was a preview. An install
; over a preview takes over its folder and retires its registration; see
; .onInit and the end of the Main section. User data is never touched here:
; the app itself migrates %LOCALAPPDATA%\Workspacer Native Rust Preview.
!define LEGACY_PRODUCT "Workspacer Native Rust Preview"
!define LEGACY_APP_ID "Workspacer.Native.RustPreview"

Name "${PRODUCT}"
OutFile "${OUTPUT}"
; Left empty so .onInit can tell an explicit /D=<folder> (in-app updates pass
; the existing install folder) from a default install, and choose the default.
InstallDir ""
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "${PRODUCT}"
Icon "${STAGE}\icon.ico"
!define MUI_ABORTWARNING
!define MUI_ICON "${STAGE}\icon.ico"
!define MUI_UNICON "${STAGE}\icon.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\wks-native.exe"
!define MUI_FINISHPAGE_RUN_PARAMETERS "--local"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"
!define LEGACY_UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${LEGACY_PRODUCT}"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "${PRODUCT} requires 64-bit Windows."
    Abort
  ${EndIf}
  SetShellVarContext current
  ; Without /D=, install over what is already there: this product's recorded
  ; folder, else a preview install's folder (an in-place upgrade from the
  ; preview identity), else the default.
  ${If} $INSTDIR == ""
    ReadRegStr $INSTDIR HKCU "Software\${PRODUCT}" "InstallDir"
  ${EndIf}
  ${If} $INSTDIR == ""
    ReadRegStr $0 HKCU "Software\${LEGACY_PRODUCT}" "InstallDir"
    ${If} $0 != ""
    ${AndIf} ${FileExists} "$0\wks-native.exe"
      StrCpy $INSTDIR $0
    ${EndIf}
  ${EndIf}
  ${If} $INSTDIR == ""
    StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCT}"
  ${EndIf}
  ClearErrors
FunctionEnd

Section "${PRODUCT}" Main
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ; The updater starts this the moment the app exits, and Windows (or an AV
  ; scan of the just-closed image) can hold wks-native.exe a few seconds
  ; longer. A silent NSIS File skips a locked target, so retry the whole
  ; payload for ~15s instead of failing on the first sharing violation.
  StrCpy $R0 0
  payload:
  ClearErrors
  File /r /x build-stamp.json "${STAGE}\*"
  ${If} ${Errors}
    IntOp $R0 $R0 + 1
    ${If} $R0 < 30
      Sleep 500
      Goto payload
    ${EndIf}
    SetErrorLevel 1
    Abort "Could not install all files. Close ${PRODUCT} and retry."
  ${EndIf}
  ; The stamp goes last: the app and updater read it as the installed
  ; version, so a partial payload must leave the previous stamp in place.
  ClearErrors
  File "${STAGE}\build-stamp.json"
  ${If} ${Errors}
    SetErrorLevel 1
    Abort "Could not install all files. Close ${PRODUCT} and retry."
  ${EndIf}
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\wks-native.exe" "--local" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "Software\${PRODUCT}" "InstallDir" "$INSTDIR"
  ; Give native toasts their own identity instead of PowerShell's default.
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${APP_ID}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${APP_ID}" "IconUri" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Workspacer"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ; Builds that still raise toasts under the preview identity keep their name
  ; and icon. Drop LEGACY_APP_ID once no supported build uses it.
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${LEGACY_APP_ID}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${LEGACY_APP_ID}" "IconUri" "$INSTDIR\icon.ico"
  ; Retire a preview registration that this install replaced in place, so
  ; Windows lists one app. The preview's uninstaller is not run (this folder's
  ; Uninstall.exe was just rewritten) and no user data is moved or deleted. A
  ; preview installed in a different folder is still a working install, so its
  ; registration and shortcuts stay.
  ReadRegStr $0 HKCU "Software\${LEGACY_PRODUCT}" "InstallDir"
  ${If} $0 == ""
    ReadRegStr $0 HKCU "${LEGACY_UNINSTALL_KEY}" "InstallLocation"
  ${EndIf}
  ${If} $0 == $INSTDIR
    DeleteRegKey HKCU "${LEGACY_UNINSTALL_KEY}"
    DeleteRegKey HKCU "Software\${LEGACY_PRODUCT}"
    Delete "$SMPROGRAMS\${LEGACY_PRODUCT}.lnk"
    Delete "$DESKTOP\${LEGACY_PRODUCT}.lnk"
  ${EndIf}
  ClearErrors
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ClearErrors
  !include "${UNINSTALL_FILES}"
  ; Nonempty directories can contain user files, so only file deletion errors
  ; are fatal (the generated include checks each Delete).
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\${PRODUCT}"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${APP_ID}"
  ; Only when no preview install in another folder still owns it.
  ReadRegStr $0 HKCU "Software\${LEGACY_PRODUCT}" "InstallDir"
  ${If} $0 == ""
    DeleteRegKey HKCU "Software\Classes\AppUserModelId\${LEGACY_APP_ID}"
  ${EndIf}
  ; Session databases, shared config and user-created files are retained:
  ; nothing under %LOCALAPPDATA%\${PRODUCT} or %LOCALAPPDATA%\${LEGACY_PRODUCT}
  ; (the app's data folders) is ever deleted here.
SectionEnd
