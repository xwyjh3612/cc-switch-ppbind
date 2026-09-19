!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping running CC Switch..."
  nsExec::ExecToLog 'taskkill /F /IM "cc-switch.exe" /T'
  Pop $0
  Sleep 1000
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'taskkill /F /IM "cc-switch.exe" /T'
  Pop $0
  Sleep 1000
!macroend
