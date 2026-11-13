; Warden's installer hooks (Tauri NSIS). Uninstalling also removes Start with Windows, so Windows does not try to
; start a program that is gone. The value is the one src/autostart.rs writes.
!macro NSIS_HOOK_PREUNINSTALL
  ; An installer update leaves hooks for the replacement app's startup repair.
  ${If} $UpdateMode != 1
    nsExec::ExecToStack '"$INSTDIR\bin\warden.exe" unhook --data-dir "$APPDATA\Warden"'
    Pop $0
    Pop $1
    ${If} $0 != 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "Warden could not remove all its hooks. Uninstall will continue." /SD IDOK
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Warden"
!macroend
