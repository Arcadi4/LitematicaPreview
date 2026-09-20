!include "LogicLib.nsh"
!include "nsDialogs.nsh"
!include "${__FILEDIR__}\..\App\src-tauri\gen\installer-extensions.nsh"

; The pinned installer template declares this options page after Welcome.
; Keep callbacks independent of Tauri variables declared after this include.
Var LPOptionsInitialized
Var LPAssociationDialog
Var LPRegisterControl
Var LPRegisterState
Var LPSettingsControl
Var LPSettingsState
Var LPPassiveMode
Var LPSelectedExtensions
Var LPEventControl
Var LPAssociationWarning

!macro LP_DECLARE_EXTENSION ID EXTENSION
  Var LP_${ID}_Control
  Var LP_${ID}_State
!macroend
!insertmacro LP_FOREACH_EXTENSION LP_DECLARE_EXTENSION

!macro LP_INITIALIZE_EXTENSION ID EXTENSION
  StrCpy $LP_${ID}_State ${BST_CHECKED}
!macroend

Function LPInitializeOptions
  ${If} $LPOptionsInitialized != 1
    StrCpy $LPOptionsInitialized 1
    StrCpy $LPRegisterState ${BST_CHECKED}
    StrCpy $LPSettingsState ${BST_UNCHECKED}
    StrCpy $LPSelectedExtensions ""
    StrCpy $LPPassiveMode 0
    !insertmacro LP_FOREACH_EXTENSION LP_INITIALIZE_EXTENSION
    Push $0
    ClearErrors
    ${GetOptions} $CMDLINE "/P" $0
    ${IfNot} ${Errors}
      StrCpy $LPPassiveMode 1
    ${EndIf}
    ClearErrors
    Pop $0
  ${EndIf}
FunctionEnd

!macro LP_CREATE_EXTENSION ID EXTENSION
  !define /math LP_EXTENSION_ROW ${LP_EXTENSION_INDEX} / 2
  !define /math LP_EXTENSION_Y ${LP_EXTENSION_ROW} * 16
  !define /redef /math LP_EXTENSION_Y ${LP_EXTENSION_Y} + 53
  !define /math LP_EXTENSION_COLUMN ${LP_EXTENSION_INDEX} % 2
  !if ${LP_EXTENSION_COLUMN} == 0
    ${NSD_CreateCheckbox} 10u ${LP_EXTENSION_Y}u 130u 12u "${EXTENSION}"
  !else
    ${NSD_CreateCheckbox} 155u ${LP_EXTENSION_Y}u 130u 12u "${EXTENSION}"
  !endif
  Pop $LP_${ID}_Control
  ${NSD_SetState} $LP_${ID}_Control $LP_${ID}_State
  ${NSD_OnClick} $LP_${ID}_Control LPOptionsChanged
  !define /redef /math LP_EXTENSION_INDEX ${LP_EXTENSION_INDEX} + 1
  !undef LP_EXTENSION_ROW
  !undef LP_EXTENSION_Y
  !undef LP_EXTENSION_COLUMN
!macroend

!macro LP_ENABLE_EXTENSION ID EXTENSION
  EnableWindow $LP_${ID}_Control $LPRegisterState
!macroend

!macro LP_CAPTURE_EXTENSION ID EXTENSION
  ${NSD_GetState} $LP_${ID}_Control $LP_${ID}_State
  ${If} $LP_${ID}_State == ${BST_CHECKED}
    ${If} $LPSelectedExtensions == ""
      StrCpy $LPSelectedExtensions "${EXTENSION}"
    ${Else}
      StrCpy $LPSelectedExtensions "$LPSelectedExtensions,${EXTENSION}"
    ${EndIf}
  ${EndIf}
!macroend

Function LPCaptureOptions
  ${NSD_GetState} $LPRegisterControl $LPRegisterState
  ${NSD_GetState} $LPSettingsControl $LPSettingsState
  StrCpy $LPSelectedExtensions ""
  !insertmacro LP_FOREACH_EXTENSION LP_CAPTURE_EXTENSION
FunctionEnd

Function LPUpdateEnabledControls
  !insertmacro LP_FOREACH_EXTENSION LP_ENABLE_EXTENSION
  EnableWindow $LPSettingsControl $LPRegisterState
FunctionEnd

Function LPOptionsChanged
  Pop $LPEventControl
  ; Capture on every click so returning with Back restores the user's choices.
  Call LPCaptureOptions
  Call LPUpdateEnabledControls
FunctionEnd


Function LPAssociationPage
  Call LPInitializeOptions
  ${If} ${Silent}
  ${OrIf} $LPPassiveMode == 1
    Abort
  ${EndIf}
  ; Silent/passive installs retain their non-interactive opt-out behavior.

  !insertmacro MUI_HEADER_TEXT "Choose file associations" "Optional settings for Litematica Preview"
  nsDialogs::Create 1018
  Pop $LPAssociationDialog
  ${If} $LPAssociationDialog == error
    MessageBox MB_OK|MB_ICONSTOP "Setup could not show the file-association options. Please run setup again."
    Quit
  ${EndIf}

  ${NSD_CreateLabel} 0 0 100% 28u "Windows keeps your current default apps. Add Litematica Preview to Open with for selected formats, then choose defaults in Windows Settings."
  Pop $LPEventControl
  ${NSD_CreateCheckbox} 0 34u 100% 12u "Register Litematica Preview for these file types"
  Pop $LPRegisterControl
  ${NSD_SetState} $LPRegisterControl $LPRegisterState
  ${NSD_OnClick} $LPRegisterControl LPOptionsChanged

  !define LP_EXTENSION_INDEX 0
  !insertmacro LP_FOREACH_EXTENSION LP_CREATE_EXTENSION
  !undef LP_EXTENSION_INDEX

  ${NSD_CreateCheckbox} 0 124u 100% 16u "Open Default Apps Settings after installation"
  Pop $LPSettingsControl
  ${NSD_SetState} $LPSettingsControl $LPSettingsState
  ${NSD_OnClick} $LPSettingsControl LPOptionsChanged
  Call LPUpdateEnabledControls
  nsDialogs::Show
FunctionEnd

Function LPAssociationPageLeave
  Call LPCaptureOptions
  ${If} $LPRegisterState == ${BST_CHECKED}
  ${AndIf} $LPSelectedExtensions == ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "Select at least one file type, or uncheck registration to continue."
    Abort
  ${EndIf}
FunctionEnd

Function LPShowAssociationWarning
  DetailPrint "$LPAssociationWarning"
  ${IfNot} ${Silent}
  ${AndIf} $LPPassiveMode != 1
    MessageBox MB_OK|MB_ICONEXCLAMATION "$LPAssociationWarning" /SD IDOK
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  ; Silent/passive installs have no options page and do not change defaults.
  Call LPInitializeOptions
  ${If} ${Silent}
  ${OrIf} $LPPassiveMode == 1
    StrCpy $LPRegisterState ${BST_UNCHECKED}
  ${EndIf}
!macroend

; Association ownership and Windows UserChoice handling belong to the app.
; Commands run only after installation, never while the user can cancel a page.
!macro NSIS_HOOK_POSTINSTALL
  Push $0
  ClearErrors
  ${If} $LPRegisterState == ${BST_CHECKED}
    DetailPrint "Registering selected file types: $LPSelectedExtensions"
    ExecWait '"$INSTDIR\LitematicaPreview.exe" --register-extensions "$LPSelectedExtensions"' $0
  ${Else}
    DetailPrint "File registration is off. Removing only associations owned by this installation."
    ExecWait '"$INSTDIR\LitematicaPreview.exe" --unregister' $0
  ${EndIf}
  ${If} ${Errors}
    StrCpy $LPAssociationWarning "Litematica Preview was installed, but setup could not start the file-association update. You can change file associations from the app."
    Call LPShowAssociationWarning
  ${ElseIf} $0 != 0
    DetailPrint "File-association update returned $0."
    StrCpy $LPAssociationWarning "Litematica Preview was installed, but file associations could not be fully updated. Another installed copy may own them. You can change file associations from the app."
    Call LPShowAssociationWarning
  ${Else}
    DetailPrint "File-association choices applied. Windows default-app choices were preserved."
  ${EndIf}

  ${IfNot} ${Silent}
  ${AndIf} $LPPassiveMode != 1
  ${AndIf} $LPRegisterState == ${BST_CHECKED}
  ${AndIf} $LPSettingsState == ${BST_CHECKED}
    ClearErrors
    DetailPrint "Opening Windows Default Apps Settings as requested."
    ExecWait '"$INSTDIR\LitematicaPreview.exe" --default-apps' $0
    ${If} ${Errors}
      StrCpy $LPAssociationWarning "Setup could not open Windows Settings. Open Settings > Apps > Default apps to choose Litematica Preview."
      Call LPShowAssociationWarning
    ${ElseIf} $0 != 0
      StrCpy $LPAssociationWarning "Windows Settings could not be opened. Open Settings > Apps > Default apps to choose Litematica Preview."
      Call LPShowAssociationWarning
    ${EndIf}
  ${EndIf}
  ClearErrors
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Tauri's own running-app check occurs after this hook. Check before any
  ; registry mutation so canceling removal leaves the installation untouched.
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Push $0
  ClearErrors
  ExecWait '"$INSTDIR\LitematicaPreview.exe" --unregister' $0
  ${If} ${Errors}
    Pop $0
    Abort "Could not start file-association cleanup. The app has not been removed."
  ${ElseIf} $0 != 0
    Pop $0
    Abort "Could not clean up file associations. The app has not been removed."
  ${EndIf}
  ClearErrors
  Pop $0
!macroend
