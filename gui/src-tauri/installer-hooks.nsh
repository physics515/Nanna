; NSIS installer/uninstaller hooks for Nanna.
;
; WHY THIS EXISTS
;
; Nanna ships its daemon as a Tauri sidecar (`bundle.externalBin`), which means
; `nanna-daemon.exe` is a SEPARATE running process from the app. NSIS knows how
; to close the app it installs; it has no idea the daemon exists. So updating
; over a running install died here:
;
;   Error opening file for writing:
;   C:\Users\<user>\AppData\Local\Nanna\nanna-daemon.exe
;   Abort / Retry / Ignore
;
; Every one of those three options is bad. Abort leaves the install half-done.
; Retry fails again unless someone manually stops the daemon. And "Ignore" is
; the worst of all, because it SUCCEEDS: the GUI updates, the daemon silently
; stays on the old build, and the two then disagree about the IPC protocol they
; share -- a failure that shows up later, far from its cause.

; Stop the daemon so its binary can be replaced.
;
; Graceful first, deliberately. The daemon holds an exclusive lock on nanna.db,
; so a hard kill risks tearing a write; `stop` asks it to close the database and
; exit. The force path is a fallback for the cases graceful cannot cover: a
; daemon that is wedged, one started outside the CLI, or a first install where
; the binary is not there yet.
;
; Both calls are allowed to fail. On a fresh install there is no daemon and no
; binary, and `taskkill` returns non-zero when nothing matches -- neither is an
; error, so both return codes are popped and ignored rather than checked.
!macro StopNannaDaemon
  DetailPrint "Stopping the Nanna daemon..."

  nsExec::ExecToLog '"$INSTDIR\nanna-daemon.exe" stop'
  Pop $0
  Sleep 2000

  nsExec::ExecToLog 'taskkill /F /T /IM nanna-daemon.exe'
  Pop $0
  Sleep 1000
!macroend

; Runs before files are written, which is exactly when the lock has to be gone.
!macro NSIS_HOOK_PREINSTALL
  !insertmacro StopNannaDaemon
!macroend

; The uninstaller hits the same wall for the same reason: it cannot delete a
; running executable, and would otherwise leave nanna-daemon.exe behind along
; with the install directory it lives in.
!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro StopNannaDaemon
!macroend
