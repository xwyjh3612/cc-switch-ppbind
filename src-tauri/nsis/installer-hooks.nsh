!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping running PPBind..."
  nsExec::ExecToLog 'taskkill /F /IM "ppbind.exe" /T'
  Pop $0
  Sleep 1000
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'taskkill /F /IM "ppbind.exe" /T'
  Pop $0
  Sleep 1000
!macroend
