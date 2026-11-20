//! Win32 Registry helpers for settings, taskbar theme detection, and Windows startup.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE,
    REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

use crate::bthome::format_mac;

pub const DEFAULT_INTERVAL_MINUTES: u32 = 5;
pub const ALLOWED_INTERVALS: [u32; 5] = [1, 5, 15, 30, 60];

const SETTINGS_SUBKEY_STR: &str = concat!("Software\\", env!("CARGO_PKG_NAME"), "\0");
const PKG_NAME_STR: &str = concat!(env!("CARGO_PKG_NAME"), "\0");

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

fn encode_utf16_nul(s: &str, buf: &mut [u16]) -> Option<usize> {
    let mut i = 0usize;
    for unit in s.encode_utf16() {
        if i + 1 >= buf.len() {
            return None;
        }
        buf[i] = unit;
        i += 1;
    }
    buf[i] = 0;
    Some(i + 1)
}

fn read_hkcu_dword(subkey: PCWSTR, value_name: PCWSTR) -> Option<u32> {
    let mut hkey = HKEY::default();
    let err = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey,
            None,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let _guard = RegKey(hkey);

    let mut val_type = REG_VALUE_TYPE::default();
    let mut bytes = [0u8; 4];
    let mut size = 4u32;
    let err = unsafe {
        RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut val_type),
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if err == ERROR_SUCCESS && val_type == REG_DWORD && size == 4 {
        Some(u32::from_le_bytes(bytes))
    } else {
        None
    }
}

/// Reads `IntervalMinutes` from `HKCU\Software\<CARGO_PKG_NAME>`, falling back to 5 minutes.
pub fn load_interval_minutes() -> u32 {
    let mut subkey = [0u16; 128];
    let Some(_) = encode_utf16_nul(SETTINGS_SUBKEY_STR.trim_end_matches('\0'), &mut subkey) else {
        return DEFAULT_INTERVAL_MINUTES;
    };
    match read_hkcu_dword(PCWSTR(subkey.as_ptr()), w!("IntervalMinutes")) {
        Some(v) if ALLOWED_INTERVALS.contains(&v) => v,
        _ => DEFAULT_INTERVAL_MINUTES,
    }
}

/// Saves `IntervalMinutes` (`REG_DWORD`) to `HKCU\Software\<CARGO_PKG_NAME>`.
pub fn save_interval_minutes(minutes: u32) -> bool {
    let mut subkey = [0u16; 128];
    let Some(_) = encode_utf16_nul(SETTINGS_SUBKEY_STR.trim_end_matches('\0'), &mut subkey) else {
        return false;
    };

    let mut hkey = HKEY::default();
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if err != ERROR_SUCCESS {
        return false;
    }
    let _guard = RegKey(hkey);

    let bytes = minutes.to_le_bytes();
    let err = unsafe {
        RegSetValueExW(
            hkey,
            w!("IntervalMinutes"),
            None,
            REG_DWORD,
            Some(&bytes),
        )
    };
    err == ERROR_SUCCESS
}

/// Reads `SystemUsesLightTheme` from Windows Personalize settings.
/// Returns `true` when the taskbar uses a light theme (dark symbols needed).
pub fn is_taskbar_light_theme() -> bool {
    read_hkcu_dword(
        w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w!("SystemUsesLightTheme"),
    ) == Some(1)
}

/// Builds a NUL-terminated UTF-16 startup command line into `out`:
/// `"C:\path\to\app.exe"` or `"C:\path\to\app.exe" --mac AA:BB:CC:DD:EE:FF`.
///
/// Returns total UTF-16 code units written (including trailing NUL), or `None` if `out` is too small.
pub fn build_autorun_command(
    exe_path_utf16: &[u16],
    cli_mac: Option<u64>,
    out: &mut [u16],
) -> Option<usize> {
    if exe_path_utf16.is_empty() {
        return None;
    }

    let mut pos = 0usize;
    let mut push = |unit: u16| -> bool {
        if pos + 1 >= out.len() {
            return false;
        }
        out[pos] = unit;
        pos += 1;
        true
    };

    if !push(b'"' as u16) {
        return None;
    }
    for &u in exe_path_utf16 {
        if u == 0 {
            break;
        }
        if !push(u) {
            return None;
        }
    }
    if !push(b'"' as u16) {
        return None;
    }

    if let Some(mac) = cli_mac {
        for &b in b" --mac " {
            if !push(b as u16) {
                return None;
            }
        }
        let mut mac_buf = [0u8; 17];
        let mac_str = format_mac(mac, &mut mac_buf);
        for &b in mac_str.as_bytes() {
            if !push(b as u16) {
                return None;
            }
        }
    }

    out[pos] = 0;
    Some(pos + 1)
}

/// Checks whether the application entry exists under `HKCU\...\Run`.
pub fn is_autorun_enabled() -> bool {
    let mut val_name = [0u16; 64];
    let Some(_) = encode_utf16_nul(PKG_NAME_STR.trim_end_matches('\0'), &mut val_name) else {
        return false;
    };

    let mut hkey = HKEY::default();
    let err = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if err != ERROR_SUCCESS {
        return false;
    }
    let _guard = RegKey(hkey);

    let err = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(val_name.as_ptr()),
            None,
            None,
            None,
            None,
        )
    };
    err == ERROR_SUCCESS
}

/// Enables or disables startup with Windows under `HKCU\...\Run`.
pub fn set_autorun(enabled: bool, cli_mac: Option<u64>) -> bool {
    let mut val_name = [0u16; 64];
    let Some(_) = encode_utf16_nul(PKG_NAME_STR.trim_end_matches('\0'), &mut val_name) else {
        return false;
    };

    let mut hkey = HKEY::default();
    let err = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            None,
            KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if err != ERROR_SUCCESS {
        return false;
    }
    let _guard = RegKey(hkey);

    if !enabled {
        let err: WIN32_ERROR = unsafe { RegDeleteValueW(hkey, PCWSTR(val_name.as_ptr())) };
        return err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND;
    }

    let mut exe_buf = [0u16; 512];
    let exe_len = unsafe { GetModuleFileNameW(None, &mut exe_buf) } as usize;
    if exe_len == 0 || exe_len >= exe_buf.len() {
        return false;
    }

    let mut cmd_buf = [0u16; 560];
    let Some(total_units) = build_autorun_command(&exe_buf[..exe_len], cli_mac, &mut cmd_buf)
    else {
        return false;
    };

    let byte_slice = unsafe {
        core::slice::from_raw_parts(cmd_buf.as_ptr() as *const u8, total_units * 2)
    };
    let err = unsafe {
        RegSetValueExW(
            hkey,
            PCWSTR(val_name.as_ptr()),
            None,
            REG_SZ,
            Some(byte_slice),
        )
    };
    err == ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn from_utf16_nul(buf: &[u16], len: usize) -> String {
        assert_eq!(buf[len - 1], 0);
        String::from_utf16(&buf[..len - 1]).unwrap()
    }

    #[test]
    fn autorun_command_with_spaces_no_mac() {
        let exe = to_utf16(r"C:\Program Files\BTHome Tray\bthome-tray.exe");
        let mut out = [0u16; 256];
        let len = build_autorun_command(&exe, None, &mut out).expect("fits in buffer");
        assert_eq!(
            from_utf16_nul(&out, len),
            r#""C:\Program Files\BTHome Tray\bthome-tray.exe""#
        );
    }

    #[test]
    fn autorun_command_with_spaces_and_mac() {
        let exe = to_utf16(r"C:\My Tools\bthome-tray.exe");
        let mut out = [0u16; 256];
        let mac = 0x0000_A4C1_3812_3456;
        let len = build_autorun_command(&exe, Some(mac), &mut out).expect("fits in buffer");
        assert_eq!(
            from_utf16_nul(&out, len),
            r#""C:\My Tools\bthome-tray.exe" --mac A4:C1:38:12:34:56"#
        );
    }

    #[test]
    fn autorun_command_rejects_tiny_buffer() {
        let exe = to_utf16(r"C:\bthome-tray.exe");
        let mut tiny = [0u16; 10];
        assert_eq!(build_autorun_command(&exe, None, &mut tiny), None);
    }
}
