; Hooks the Tauri NSIS installer runs. context-menu.wxs registers the same
; menu entries for the MSI.

; "Scan with Reclaim" in Explorer's right-click menu: on folders, inside a
; folder and on drives. %1 is the item clicked, %V the folder being shown.
; On Windows 11 it's under "Show more options".
!macro ReclaimScanVerb KEY FOLDER
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\ReclaimScan" "" "Scan with Reclaim"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\ReclaimScan" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\ReclaimScan" "MultiSelectModel" "Single"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\ReclaimScan\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --scan $\"${FOLDER}$\""
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro ReclaimScanVerb "Directory" "%1"
  !insertmacro ReclaimScanVerb "Directory\Background" "%V"
  !insertmacro ReclaimScanVerb "Drive" "%1"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey SHCTX "Software\Classes\Directory\shell\ReclaimScan"
  DeleteRegKey SHCTX "Software\Classes\Directory\Background\shell\ReclaimScan"
  DeleteRegKey SHCTX "Software\Classes\Drive\shell\ReclaimScan"
!macroend
