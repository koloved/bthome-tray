#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ble;
mod bthome;
mod icon;
mod registry;

use core::ffi::c_void;
use std::time::Instant;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT,
    POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize,
};
use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW,
    HICON, KillTimer, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_CREATE, WM_DESTROY, WM_NULL,
    WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use ble::{BleScanner, WM_APP_BLE_PACKET};
use bthome::{BthomeReading, format_icon_text, format_tooltip_utf16, is_reading_stale, parse_mac};

const TRAY_ICON_ID: u32 = 1;
const WM_APP_TRAY: u32 = WM_APP + 2;

const TIMER_PERIODIC_ID: usize = 1;
const TIMER_SCAN_TIMEOUT_ID: usize = 2;
const INITIAL_SCAN_TIMEOUT_MS: u32 = 120_000;
const SCAN_TIMEOUT_MS: u32 = 50_000;

const IDM_CHECK_NOW: usize = 1001;
const IDM_INTERVAL_1: usize = 2001;
const IDM_INTERVAL_5: usize = 2005;
const IDM_INTERVAL_15: usize = 2015;
const IDM_INTERVAL_30: usize = 2030;
const IDM_INTERVAL_60: usize = 2060;
const IDM_AUTOSTART: usize = 3001;
const IDM_EXIT: usize = 4001;

struct CliArgs {
    mac: Option<u64>,
    interval_minutes: Option<u32>,
}

fn parse_cli_args() -> CliArgs {
    let mut parsed = CliArgs {
        mac: None,
        interval_minutes: None,
    };

    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let Some(s) = arg.to_str() else {
            continue;
        };
        if s == "--mac" {
            if let Some(val) = args.next()
                && let Some(val_str) = val.to_str()
            {
                parsed.mac = parse_mac(val_str);
            }
        } else if s == "--interval"
            && let Some(val) = args.next()
            && let Some(val_str) = val.to_str()
            && let Ok(mins) = val_str.parse::<u32>()
            && (1..=1440).contains(&mins)
        {
            parsed.interval_minutes = Some(mins);
        }
    }
    parsed
}

struct SingleInstanceMutex(HANDLE);

impl SingleInstanceMutex {
    fn acquire() -> Option<Self> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\bthome-tray-single-instance")) }
            .ok()?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
        Some(Self(handle))
    }
}

impl Drop for SingleInstanceMutex {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct WinRtGuard;

impl WinRtGuard {
    fn init() -> windows::core::Result<Self> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }?;
        Ok(Self)
    }
}

impl Drop for WinRtGuard {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

struct AppState {
    cli_mac: Option<u64>,
    interval_minutes: u32,
    scanner: Option<BleScanner>,
    scanning: bool,
    last_reading: Option<BthomeReading>,
    last_received_at: Option<Instant>,
    current_icon: HICON,
    rendered_text: [u8; 8],
    rendered_text_len: usize,
    rendered_light_theme: bool,
    rendered_icon_size: (i32, i32),
    last_tip: [u16; 128],
    taskbar_created_msg: u32,
}

impl AppState {
    fn new(cli_mac: Option<u64>, interval_minutes: u32, taskbar_created_msg: u32) -> Self {
        Self {
            cli_mac,
            interval_minutes,
            scanner: None,
            scanning: false,
            last_reading: None,
            last_received_at: None,
            current_icon: HICON::default(),
            rendered_text: [0u8; 8],
            rendered_text_len: 0,
            rendered_light_theme: false,
            rendered_icon_size: (0, 0),
            last_tip: [0u16; 128],
            taskbar_created_msg,
        }
    }

    fn is_stale(&self) -> bool {
        match self.last_received_at {
            Some(at) => is_reading_stale(at.elapsed().as_secs(), self.interval_minutes),
            None => false,
        }
    }
}

fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

fn update_tray_icon(hwnd: HWND, state: &mut AppState, force_add: bool) {
    let stale = state.is_stale();
    let temp_centi = state.last_reading.as_ref().and_then(|r| r.temp_centi);
    let mut text_buf = [0u8; 8];
    let text = format_icon_text(temp_centi, &mut text_buf);

    let light_theme = registry::is_taskbar_light_theme();
    let icon_size = icon::small_icon_size();

    let need_new_icon = state.current_icon.is_invalid()
        || text.as_bytes() != &state.rendered_text[..state.rendered_text_len]
        || light_theme != state.rendered_light_theme
        || icon_size != state.rendered_icon_size;

    let mut old_icon = HICON::default();
    if need_new_icon && let Some(new_icon) = icon::render_temp_icon(text, light_theme) {
        old_icon = state.current_icon;
        state.current_icon = new_icon;
        let bytes = text.as_bytes();
        let copy_len = bytes.len().min(state.rendered_text.len());
        state.rendered_text[..copy_len].copy_from_slice(&bytes[..copy_len]);
        state.rendered_text_len = copy_len;
        state.rendered_light_theme = light_theme;
        state.rendered_icon_size = icon_size;
    }

    let mut tip_buf = [0u16; 128];
    format_tooltip_utf16(
        state.last_reading.as_ref(),
        stale,
        state.interval_minutes,
        &mut tip_buf,
    );
    let tip_changed = tip_buf != state.last_tip;

    if force_add || need_new_icon || tip_changed {
        let mut flags = NIF_TIP;
        if force_add {
            flags |= NIF_MESSAGE | NIF_ICON;
        } else if need_new_icon {
            flags |= NIF_ICON;
        }

        let nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: flags,
            uCallbackMessage: WM_APP_TRAY,
            hIcon: state.current_icon,
            szTip: tip_buf,
            ..Default::default()
        };

        let msg = if force_add { NIM_ADD } else { NIM_MODIFY };
        unsafe {
            let _ = Shell_NotifyIconW(msg, &nid);
        }
        state.last_tip = tip_buf;
    }

    if !old_icon.is_invalid() {
        icon::destroy_icon(old_icon);
    }
}

fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn begin_scan(hwnd: HWND, state: &mut AppState) {
    if state.scanning {
        return;
    }
    if let Some(scanner) = state.scanner.as_ref()
        && scanner.start()
    {
        state.scanning = true;
        let timeout_ms = if state.last_reading.is_none() {
            INITIAL_SCAN_TIMEOUT_MS
        } else {
            SCAN_TIMEOUT_MS
        };
        unsafe {
            let _ = SetTimer(Some(hwnd), TIMER_SCAN_TIMEOUT_ID, timeout_ms, None);
        }
    } else {
        trim_working_set();
    }
}

fn finish_scan(hwnd: HWND, state: &mut AppState) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_SCAN_TIMEOUT_ID);
    }
    if let Some(scanner) = state.scanner.as_ref() {
        scanner.stop();
    }
    state.scanning = false;
    trim_working_set();
}

fn set_periodic_timer(hwnd: HWND, interval_minutes: u32) {
    let ms = interval_minutes.saturating_mul(60_000);
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_PERIODIC_ID, ms, None);
    }
}

fn show_context_menu(hwnd: HWND, state: &AppState) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let Ok(interval_submenu) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };

        let interval_items: [(usize, u32, PCWSTR); 5] = [
            (IDM_INTERVAL_1, 1, w!("1 minute")),
            (IDM_INTERVAL_5, 5, w!("5 minutes")),
            (IDM_INTERVAL_15, 15, w!("15 minutes")),
            (IDM_INTERVAL_30, 30, w!("30 minutes")),
            (IDM_INTERVAL_60, 60, w!("60 minutes")),
        ];
        for (id, mins, label) in interval_items {
            let flags = if state.interval_minutes == mins {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING | MF_UNCHECKED
            };
            let _ = AppendMenuW(interval_submenu, flags, id, label);
        }

        let _ = AppendMenuW(menu, MF_STRING, IDM_CHECK_NOW, w!("Check Now"));
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            interval_submenu.0 as usize,
            w!("Check Interval"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);

        let autostart_flags = if registry::is_autorun_enabled() {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING | MF_UNCHECKED
        };
        let _ = AppendMenuW(menu, autostart_flags, IDM_AUTOSTART, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        return LRESULT(0);
    }

    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppState;
    if state_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let state = unsafe { &mut *state_ptr };

    if state.taskbar_created_msg != 0 && msg == state.taskbar_created_msg {
        update_tray_icon(hwnd, state, true);
        trim_working_set();
        return LRESULT(0);
    }

    match msg {
        WM_APP_BLE_PACKET => {
            finish_scan(hwnd, state);
            if let Some(reading) = ble::take_captured_reading() {
                state.last_reading = Some(reading);
                state.last_received_at = Some(Instant::now());
                update_tray_icon(hwnd, state, false);
                trim_working_set();
            }
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_PERIODIC_ID => {
                    update_tray_icon(hwnd, state, false);
                    begin_scan(hwnd, state);
                }
                TIMER_SCAN_TIMEOUT_ID => {
                    finish_scan(hwnd, state);
                    update_tray_icon(hwnd, state, false);
                    trim_working_set();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let event = (lparam.0 as u32) & 0xFFFF;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                show_context_menu(hwnd, state);
                trim_working_set();
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = wparam.0 & 0xFFFF;
            match cmd {
                IDM_CHECK_NOW => {
                    begin_scan(hwnd, state);
                }
                IDM_INTERVAL_1 | IDM_INTERVAL_5 | IDM_INTERVAL_15 | IDM_INTERVAL_30
                | IDM_INTERVAL_60 => {
                    let mins = match cmd {
                        IDM_INTERVAL_1 => 1,
                        IDM_INTERVAL_5 => 5,
                        IDM_INTERVAL_15 => 15,
                        IDM_INTERVAL_30 => 30,
                        _ => 60,
                    };
                    state.interval_minutes = mins;
                    let _ = registry::save_interval_minutes(mins);
                    set_periodic_timer(hwnd, mins);
                    update_tray_icon(hwnd, state, false);
                    begin_scan(hwnd, state);
                }
                IDM_AUTOSTART => {
                    let enabled = registry::is_autorun_enabled();
                    let _ = registry::set_autorun(!enabled, state.cli_mac);
                }
                IDM_EXIT => {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            update_tray_icon(hwnd, state, false);
            trim_working_set();
            LRESULT(0)
        }
        WM_DESTROY => {
            finish_scan(hwnd, state);
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_PERIODIC_ID);
            }
            state.scanner = None;
            remove_tray_icon(hwnd);
            if !state.current_icon.is_invalid() {
                icon::destroy_icon(state.current_icon);
                state.current_icon = HICON::default();
            }
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() {
    let Some(_instance_mutex) = SingleInstanceMutex::acquire() else {
        return;
    };

    let cli = parse_cli_args();
    let interval_minutes = cli
        .interval_minutes
        .unwrap_or_else(registry::load_interval_minutes);

    let Ok(_winrt_guard) = WinRtGuard::init() else {
        return;
    };

    let class_name = w!("BTHomeTrayWindowClass");
    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map(Into::into)
        .unwrap_or_default();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        lpszClassName: class_name,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        return;
    }

    let taskbar_created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let mut state = AppState::new(cli.mac, interval_minutes, taskbar_created_msg);

    let Ok(hwnd) = (unsafe {
        CreateWindowExW(
            Default::default(),
            class_name,
            w!("BTHome Tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            Some((&mut state as *mut AppState).cast::<c_void>()),
        )
    }) else {
        return;
    };

    state.scanner = BleScanner::new(hwnd, state.cli_mac).ok();
    update_tray_icon(hwnd, &mut state, true);
    set_periodic_timer(hwnd, state.interval_minutes);
    trim_working_set();

    // Run an initial BLE scan burst right on startup.
    begin_scan(hwnd, &mut state);

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
