# Uninstalling Nanna

## Windows

### Remove Start Menu shortcuts and taskbar icon

1. Open **Settings → Apps → Installed apps** (or `Win + I` → Apps).
2. Search for **Nanna** and click **Uninstall**.
3. Confirm the removal.

### Stop the daemon service

The Nanna daemon runs as a Windows service. Stop it before uninstalling:

```powershell
Stop-Service -Name "NannaDaemon" -Force
```

If the service name differs, find it with:

```powershell
Get-Service | Where-Object {$_.DisplayName -like "*Nanna*"}
```

### Remove launch-at-login entries

Check these locations and delete any Nanna-related entries:

- `shell:startup` — `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`
- Task Scheduler (`taskschd.msc`) — remove any tasks named `Nanna*`
- Registry: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` — delete the `Nanna` value

### Remove residual files

```powershell
Remove-Item "$env:APPDATA\clawd\Nanna" -Recurse -Force
Remove-Item "$env:LOCALAPPDATA\Programs\Nanna" -Recurse -Force
```

### Uninstall via installer (recommended)

If you installed with the `.exe` or `.msi`, run the uninstaller from the same location. The bundled daemon sidecar is removed automatically.

---

## macOS

### Quit Nanna

Quit the app from the menu bar (`Cmd + Q`) or Activity Monitor.

### Remove launchd entries

```bash
launchctl unload ~/Library/LaunchAgents/com.nanna.daemon.plist 2>/dev/null
rm ~/Library/LaunchAgents/com.nanna.daemon.plist
```

Check for any other Nanna plist files:

```bash
ls ~/Library/LaunchAgents/*nanna* 2>/dev/null
```

### Remove the app

```bash
rm -rf ~/Applications/Nanna.app
```

Or drag `Nanna.app` to Trash and empty it.

### Remove residual files

```bash
rm -rf ~/Library/Application\ Support/clawd/Nanna
rm -rf ~/Library/Caches/Nanna
```

### Uninstall the DMG installer

The `.dmg` itself is just a disk image — no cleanup needed beyond removing the app and support folders above. If you want to remove the DMG file from `/Applications`, just delete it.

---

## Linux

### AppImage

1. Remove the launcher shortcut:

```bash
rm -f ~/.local/share/applications/nanna.desktop
```

2. Delete the AppImage binary:

```bash
rm -f ~/Downloads/Nanna_x.y.z.AppImage  # or wherever you placed it
chmod -x ~/Downloads/Nanna_x.y.z.AppImage  # remove execute permission just in case
```

3. Remove desktop integration files:

```bash
rm -f ~/.config/autostart/nanna.desktop
rm -f ~/.config/autostart/nanna.service
```

### .deb package

```bash
# Stop the daemon
sudo systemctl stop nanna-daemon
sudo systemctl disable nanna-daemon

# Remove system-wide files
sudo dpkg -r nanna
sudo rm -rf /usr/share/applications/nanna.desktop
sudo rm -rf /usr/share/icons/hicolor/*/apps/nanna.*
```

### .rpm package

```bash
sudo rpm -e --nodeps nanna
sudo rm -f ~/.config/autostart/nanna.desktop
```

### Flatpak (if applicable)

```bash
flatpak uninstall com.nanna.Nanna
```

### Snap (if applicable)

```bash
snap remove nanna
```

### Clean up residual files

```bash
rm -rf ~/.local/share/Nanna
rm -rf ~/.cache/Nanna
```

---

## Verifying removal

Run these to confirm nothing remains:

- **Windows**: `Get-Service | Where-Object {$_.DisplayName -like "*Nanna*"}` — should return nothing.
- **macOS**: `launchctl list | grep nanna` — should return nothing.
- **Linux**: `systemctl status nanna-daemon` — should report "not found" or "inactive".
