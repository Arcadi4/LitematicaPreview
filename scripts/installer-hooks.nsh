!include "LogicLib.nsh"

; Association ownership and UserChoice handling belong exclusively to the app.
; ExecWait keeps registration behind file installation and removal behind cleanup.
!macro NSIS_HOOK_POSTINSTALL
  Push $0
  ClearErrors
  ExecWait '"$INSTDIR\LitematicaPreview.exe" --register' $0
  ${If} ${Errors}
    DetailPrint "Could not start file-association registration."
    MessageBox MB_OK|MB_ICONEXCLAMATION "Litematica Preview was installed, but file associations could not be registered. You can register them from the app." /SD IDOK
  ${ElseIf} $0 != 0
    DetailPrint "File-association registration returned $0; existing associations were preserved."
    MessageBox MB_OK|MB_ICONEXCLAMATION "Litematica Preview was installed, but file associations could not be registered. Another installed copy may own them. Existing associations were preserved." /SD IDOK
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
