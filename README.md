# bthome-tray

Ultra-lightweight native Windows 11 system tray utility for passively receiving **BTHome v2** BLE advertisements (e.g. Xiaomi LYWSD03MMC running `pvvx` firmware) and displaying the current temperature directly on the tray icon.

## Features

- **Minimal footprint:** uses only the official `windows` crate (WinRT + Win32 API) with no third-party runtimes or extra dependencies (~134 KB release binary, `< 1 MB` Private Working Set in idle, 0% CPU between checks).
- **Burst passive scanning:** runs `BluetoothLEAdvertisementWatcher` in `Passive` mode on a timer, immediately stops the watcher upon receiving the first valid unencrypted temperature packet (or on timeout: 50 s, up to 120 s on initial startup acquisition), and trims the process working set (`SetProcessWorkingSetSize`).
- **DPI- and theme-aware tray icon:** renders a crisp 32-bit ARGB pixel-font icon (`7×16` bold / `5×12` compact) sized to `SM_CXSMICON`/`SM_CYSMICON` and adapted to the Windows 11 taskbar theme (`SystemUsesLightTheme`). Displays `--` before the first reading or after 2 consecutive missed checks (`> interval * 2 + 1` min).
- **Single-instance:** prevents duplicate instances using a named Win32 mutex.

## Build

```powershell
cargo build --release
```

Output binary: `target\release\bthome-tray.exe`.

## Usage & CLI Flags

```powershell
.\target\release\bthome-tray.exe [--mac AA:BB:CC:DD:EE:FF] [--interval 5]
```

- `--mac <AA:BB:CC:DD:EE:FF>` — pin a specific BTHome v2 sensor MAC address. If omitted, the utility locks onto the MAC address of the first valid sensor broadcasting temperature for the current session.
- `--interval <minutes>` — polling interval in minutes for the current session (overrides the registry setting without overwriting it).

Interval selections from the tray menu are persisted under `HKCU\Software\bthome-tray` (`IntervalMinutes`, `REG_DWORD`). Default: **5 minutes**.

## Tray Context Menu (Right-Click)

- **Check Now** — immediately starts an out-of-schedule BLE scan burst.
- **Check Interval** — choose the polling period: `1 minute`, `5 minutes`, `15 minutes`, `30 minutes`, or `60 minutes`. Saves the selection to the registry and triggers an immediate check.
- **Start with Windows** — toggles startup via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (preserving `--mac` if passed at launch).
- **Exit** — stops the BLE watcher, removes the tray icon, and exits.
