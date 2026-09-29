Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!ifdef RUST_PREVIEW
!define PRODUCT "Workspacer Native Rust Preview"
!define APP_ID "Workspacer.Native.RustPreview"
!else
!define PRODUCT "Workspacer Native"
!define APP_ID "Workspacer.Native"
!endif

Name "${PRODUCT}"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
InstallDirRegKey HKCU "Software\${PRODUCT}" "InstallDir"
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

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "${PRODUCT} requires 64-bit Windows."
    Abort
  ${EndIf}
  SetShellVarContext current
FunctionEnd

Section "${PRODUCT}" Main
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ClearErrors
  File /r "${STAGE}\*"
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
  ; Session databases, shared config and user-created files are retained.
SectionEnd
