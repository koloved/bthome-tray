//! Passive burst-mode BLE watcher using WinRT `BluetoothLEAdvertisementWatcher`.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEScanningMode,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::DataReader;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{Ref, Result};

use crate::bthome::{BLE_SERVICE_DATA_16_TYPE, BthomeReading, parse_service_data};

pub const WM_APP_BLE_PACKET: u32 = WM_APP + 1;

const MAC_UNSET: u64 = u64::MAX;

static SCAN_ACTIVE: AtomicBool = AtomicBool::new(false);
static TARGET_MAC: AtomicU64 = AtomicU64::new(MAC_UNSET);
static NOTIFY_HWND: AtomicUsize = AtomicUsize::new(0);
static CAPTURED: Mutex<Option<BthomeReading>> = Mutex::new(None);

pub struct BleScanner {
    watcher: BluetoothLEAdvertisementWatcher,
    received_token: i64,
}

impl BleScanner {
    pub fn new(hwnd: HWND, initial_target_mac: Option<u64>) -> Result<Self> {
        NOTIFY_HWND.store(hwnd.0 as usize, Ordering::Release);
        TARGET_MAC.store(initial_target_mac.unwrap_or(MAC_UNSET), Ordering::Release);

        let watcher = BluetoothLEAdvertisementWatcher::new()?;
        watcher.SetScanningMode(BluetoothLEScanningMode::Passive)?;
        let _ = watcher.SetAllowExtendedAdvertisements(true);

        let handler = TypedEventHandler::<
            BluetoothLEAdvertisementWatcher,
            BluetoothLEAdvertisementReceivedEventArgs,
        >::new(|_sender, args: Ref<'_, BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(args) = args.as_ref() {
                on_advertisement_received(args)?;
            }
            Ok(())
        });

        let received_token = watcher.Received(&handler)?;
        Ok(Self {
            watcher,
            received_token,
        })
    }

    pub fn start(&self) -> bool {
        SCAN_ACTIVE.store(true, Ordering::Release);
        if self.watcher.Start().is_err() {
            SCAN_ACTIVE.store(false, Ordering::Release);
            return false;
        }
        true
    }

    pub fn stop(&self) {
        SCAN_ACTIVE.store(false, Ordering::Release);
        let _ = self.watcher.Stop();
    }
}

impl Drop for BleScanner {
    fn drop(&mut self) {
        self.stop();
        let _ = self.watcher.RemoveReceived(self.received_token);
    }
}

pub fn take_captured_reading() -> Option<BthomeReading> {
    CAPTURED.lock().ok().and_then(|mut guard| guard.take())
}

fn on_advertisement_received(args: &BluetoothLEAdvertisementReceivedEventArgs) -> Result<()> {
    if !SCAN_ACTIVE.load(Ordering::Acquire) {
        return Ok(());
    }

    let mac = args.BluetoothAddress()?;
    let target = TARGET_MAC.load(Ordering::Acquire);
    if target != MAC_UNSET && mac != target {
        return Ok(());
    }

    let adv = args.Advertisement()?;
    let sections = adv.DataSections()?;
    let count = sections.Size()?;

    for idx in 0..count {
        let section = sections.GetAt(idx)?;
        if section.DataType()? != BLE_SERVICE_DATA_16_TYPE {
            continue;
        }

        let buf = section.Data()?;
        let reader = DataReader::FromBuffer(&buf)?;
        let len = reader.UnconsumedBufferLength()? as usize;
        if !(3..=128).contains(&len) {
            continue;
        }

        let mut raw = [0u8; 128];
        reader.ReadBytes(&mut raw[..len])?;

        if let Some(reading) = parse_service_data(&raw[..len]) {
            if reading.temp_centi.is_none() {
                continue;
            }
            if SCAN_ACTIVE.swap(false, Ordering::AcqRel) {
                let _ = TARGET_MAC.compare_exchange(
                    MAC_UNSET,
                    mac,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                if let Ok(mut guard) = CAPTURED.lock() {
                    *guard = Some(reading);
                }
                let hwnd_raw = NOTIFY_HWND.load(Ordering::Acquire);
                if hwnd_raw != 0 {
                    let hwnd = HWND(hwnd_raw as *mut c_void);
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WM_APP_BLE_PACKET, WPARAM(0), LPARAM(0));
                    }
                }
            }
            break;
        }
    }

    Ok(())
}
