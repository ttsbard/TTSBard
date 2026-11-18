!include "LogicLib.nsh"

; The template checks the same executable name immediately after this hook.
; Give an in-app update time to shut down before that check can force a close.
; The bundled wizard uses English. Literal messages avoid creating a partial
; language table before the template loads its Modern UI language.
!macro NSIS_HOOK_PREINSTALL
  ${If} $UpdateMode = 1
    Push $R0
    Push $R1
    ttsbard_update_retry:
      StrCpy $R1 0
    ttsbard_update_wait:
      !if "${INSTALLMODE}" == "currentUser"
        nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
      !else
        nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
      !endif
      Pop $R0
      StrCmp $R0 1 ttsbard_update_ready
      IntOp $R1 $R1 + 1
      ${If} $R1 >= 64
        IfSilent ttsbard_update_abort 0
        MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "TTSBard is still running. Close it and click Retry, or click Cancel to stop the update." /SD IDCANCEL IDRETRY ttsbard_update_retry IDCANCEL ttsbard_update_abort
        Goto ttsbard_update_abort
      ${EndIf}
      Sleep 250
      Goto ttsbard_update_wait
    ttsbard_update_abort:
      Pop $R1
      Pop $R0
      Abort "The update was cancelled because TTSBard did not close."
    ttsbard_update_ready:
      Pop $R1
      Pop $R0
  ${EndIf}
!macroend
