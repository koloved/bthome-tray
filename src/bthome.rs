//! Pure BTHome v2 protocol parser, staleness calculation, and stack-allocated formatters.
//!
//! This module has zero dependencies on Windows APIs so it can be unit-tested in isolation.

pub const BLE_SERVICE_DATA_16_TYPE: u8 = 0x16;
pub const BTHOME_UUID_LE: [u8; 2] = [0xD2, 0xFC];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BthomeReading {
    pub packet_id: Option<u8>,
    pub battery: Option<u8>,
    /// Temperature in 0.01 °C units.
    pub temp_centi: Option<i16>,
    /// Humidity in 0.01 % units.
    pub humidity_centi: Option<u16>,
    /// Battery voltage in 0.001 V (mV) units.
    pub voltage_mv: Option<u16>,
}

/// Parses a BLE `0x16` Service Data payload for an unencrypted BTHome v2 packet.
///
/// Expects `data` to start with the 16-bit Service UUID `0xFCD2` in little-endian (`[0xD2, 0xFC]`),
/// followed by the BTHome `Device Info` byte and sequential `[Object ID][Value]` pairs.
pub fn parse_service_data(data: &[u8]) -> Option<BthomeReading> {
    if data.len() < 3 {
        return None;
    }
    if data[0] != BTHOME_UUID_LE[0] || data[1] != BTHOME_UUID_LE[1] {
        return None;
    }

    let info = data[2];
    // Bits 7..5 encode BTHome protocol version; v2 == 2 (0b010 << 5 == 0x40).
    if (info & 0xE0) != 0x40 {
        return None;
    }
    // Bit 0 is encryption flag; encrypted packets are ignored.
    if (info & 0x01) != 0 {
        return None;
    }

    let mut reading = BthomeReading::default();
    let mut offset = 3usize;

    while offset < data.len() {
        let obj_id = data[offset];
        offset += 1;

        match obj_id {
            // 0x00: Packet ID (uint8, 1 byte)
            0x00 => {
                if offset + 1 > data.len() {
                    break;
                }
                reading.packet_id = Some(data[offset]);
                offset += 1;
            }
            // 0x01: Battery (uint8, 1 byte, 1%)
            0x01 => {
                if offset + 1 > data.len() {
                    break;
                }
                reading.battery = Some(data[offset]);
                offset += 1;
            }
            // 0x02: Temperature (sint16 LE, 2 bytes, 0.01 °C)
            0x02 => {
                if offset + 2 > data.len() {
                    break;
                }
                let raw = i16::from_le_bytes([data[offset], data[offset + 1]]);
                reading.temp_centi = Some(raw);
                offset += 2;
            }
            // 0x03: Humidity (uint16 LE, 2 bytes, 0.01 %)
            0x03 => {
                if offset + 2 > data.len() {
                    break;
                }
                let raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
                reading.humidity_centi = Some(raw);
                offset += 2;
            }
            // 0x0C: Voltage (uint16 LE, 2 bytes, 0.001 V)
            0x0C => {
                if offset + 2 > data.len() {
                    break;
                }
                let raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
                reading.voltage_mv = Some(raw);
                offset += 2;
            }
            // 0x2E: Humidity (uint8, 1 byte, 1 %)
            0x2E => {
                if offset + 1 > data.len() {
                    break;
                }
                reading.humidity_centi = Some((data[offset] as u16) * 100);
                offset += 1;
            }
            // 0x45: Temperature (sint16 LE, 2 bytes, 0.1 °C)
            0x45 => {
                if offset + 2 > data.len() {
                    break;
                }
                let raw = i16::from_le_bytes([data[offset], data[offset + 1]]);
                reading.temp_centi = Some(raw.saturating_mul(10));
                offset += 2;
            }
            // Unknown Object ID: stop parsing safely without panicking.
            _ => break,
        }
    }

    Some(reading)
}

/// Returns the staleness threshold in seconds: `interval * 2 + 1` minutes.
pub const fn stale_threshold_secs(interval_minutes: u32) -> u64 {
    (interval_minutes as u64) * 120 + 60
}

/// Returns `true` when `elapsed_secs` exceeds `interval * 2 + 1` minutes.
pub const fn is_reading_stale(elapsed_secs: u64, interval_minutes: u32) -> bool {
    elapsed_secs > stale_threshold_secs(interval_minutes)
}

/// Rounds temperature in 0.01 °C to the nearest whole degree.
pub const fn round_temp_deg(temp_centi: i16) -> i16 {
    let t = temp_centi as i32;
    if t >= 0 {
        ((t + 50) / 100) as i16
    } else {
        ((t - 50) / 100) as i16
    }
}

/// Rounds temperature in 0.01 °C to the nearest 0.1 °C (in tenths of a degree).
pub const fn round_temp_tenths(temp_centi: i16) -> i16 {
    let t = temp_centi as i32;
    if t >= 0 {
        ((t + 5) / 10) as i16
    } else {
        ((t - 5) / 10) as i16
    }
}

/// Parses a MAC address like `AA:BB:CC:DD:EE:FF` into a 48-bit big-endian `u64`.
pub fn parse_mac(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() != 17 {
        return None;
    }
    let sep = bytes[2];
    if sep != b':' && sep != b'-' {
        return None;
    }

    let mut mac: u64 = 0;
    for i in 0..6 {
        let idx = i * 3;
        if i > 0 && bytes[idx - 1] != sep {
            return None;
        }
        let hi = hex_nibble(bytes[idx])?;
        let lo = hex_nibble(bytes[idx + 1])?;
        mac = (mac << 8) | (((hi << 4) | lo) as u64);
    }
    Some(mac)
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Formats a 48-bit MAC address into `"AA:BB:CC:DD:EE:FF"` in `buf`.
pub fn format_mac(mac: u64, buf: &mut [u8; 17]) -> &str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in 0..6 {
        let byte = ((mac >> ((5 - i) * 8)) & 0xFF) as usize;
        let pos = i * 3;
        buf[pos] = HEX[byte >> 4];
        buf[pos + 1] = HEX[byte & 0x0F];
        if i < 5 {
            buf[pos + 2] = b':';
        }
    }
    // SAFETY: `buf` contains only ASCII hex digits and `':'`.
    unsafe { core::str::from_utf8_unchecked(buf) }
}

/// Formats the short text drawn on the tray icon (`"24"`, `"-5"`, or `"--"` before first reading).
pub fn format_icon_text(temp_centi: Option<i16>, buf: &mut [u8; 8]) -> &str {
    let len = match temp_centi {
        Some(tc) => {
            let deg = round_temp_deg(tc);
            write_i32_ascii(deg as i32, buf)
        }
        None => {
            buf[0] = b'-';
            buf[1] = b'-';
            2
        }
    };
    // SAFETY: `buf[..len]` contains only ASCII digits and `'-'`.
    unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
}

/// Formats the NUL-terminated UTF-16 tray tooltip into `buf` (`NOTIFYICONDATAW::szTip`).
pub fn format_tooltip_utf16(
    reading: Option<&BthomeReading>,
    stale: bool,
    interval_minutes: u32,
    buf: &mut [u16; 128],
) {
    buf.fill(0);
    let mut w = Utf16Buf::new(buf);

    let Some(r) = reading else {
        w.push_str("Scanning BTHome v2...");
        return;
    };

    if let Some(tc) = r.temp_centi {
        w.push_temp_tenths(tc);
        w.push_str(" \u{00B0}C");
    } else {
        w.push_str("-- \u{00B0}C");
    }

    if let Some(hc) = r.humidity_centi {
        let pct = ((hc as u32) + 50) / 100;
        w.push_str(" | ");
        w.push_u32(pct);
        w.push_str("%");
    }

    if let Some(bat) = r.battery {
        w.push_str(" | Bat: ");
        w.push_u32(bat as u32);
        w.push_str("%");
    } else if let Some(mv) = r.voltage_mv {
        let centi_v = ((mv as u32) + 5) / 10;
        w.push_str(" | ");
        w.push_u32(centi_v / 100);
        w.push_char('.');
        let frac = centi_v % 100;
        if frac < 10 {
            w.push_char('0');
        }
        w.push_u32(frac);
        w.push_char('V');
    }

    w.push_str(" (");
    w.push_u32(interval_minutes);
    if stale {
        w.push_str("m, stale)");
    } else {
        w.push_str("m)");
    }
}

struct Utf16Buf<'a> {
    buf: &'a mut [u16],
    pos: usize,
}

impl<'a> Utf16Buf<'a> {
    fn new(buf: &'a mut [u16]) -> Self {
        Self { buf, pos: 0 }
    }

    fn push_u16(&mut self, unit: u16) {
        // Reserve the final slot for the NUL terminator.
        if self.pos + 1 < self.buf.len() {
            self.buf[self.pos] = unit;
            self.pos += 1;
        }
    }

    fn push_char(&mut self, ch: char) {
        let mut enc = [0u16; 2];
        for u in ch.encode_utf16(&mut enc) {
            self.push_u16(*u);
        }
    }

    fn push_str(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.push_u16(u);
        }
    }

    fn push_u32(&mut self, mut v: u32) {
        let mut tmp = [0u8; 10];
        let mut len = 0usize;
        if v == 0 {
            self.push_u16(b'0' as u16);
            return;
        }
        while v > 0 {
            tmp[len] = b'0' + (v % 10) as u8;
            len += 1;
            v /= 10;
        }
        while len > 0 {
            len -= 1;
            self.push_u16(tmp[len] as u16);
        }
    }

    fn push_temp_tenths(&mut self, temp_centi: i16) {
        let tenths = round_temp_tenths(temp_centi);
        if tenths < 0 {
            self.push_u16(b'-' as u16);
        }
        let abs = tenths.unsigned_abs() as u32;
        self.push_u32(abs / 10);
        self.push_u16(b'.' as u16);
        self.push_u32(abs % 10);
    }
}

fn write_i32_ascii(val: i32, out: &mut [u8]) -> usize {
    if val == 0 {
        if !out.is_empty() {
            out[0] = b'0';
            return 1;
        }
        return 0;
    }

    let neg = val < 0;
    let mut mag = val.unsigned_abs();
    let mut tmp = [0u8; 12];
    let mut n = 0usize;
    while mag > 0 {
        tmp[n] = b'0' + (mag % 10) as u8;
        n += 1;
        mag /= 10;
    }

    let mut pos = 0usize;
    if neg && pos < out.len() {
        out[pos] = b'-';
        pos += 1;
    }
    while n > 0 && pos < out.len() {
        n -= 1;
        out[pos] = tmp[n];
        pos += 1;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_pvvx_packet() {
        // UUID 0xFCD2 LE, Info 0x40 (v2, unencrypted),
        // PacketID 0x09, Battery 92%, Temp 23.45 °C (2345 = 0x0929), Humidity 48.12% (4812 = 0x12CC)
        let data = [
            0xD2, 0xFC, 0x40, 0x00, 0x09, 0x01, 0x5C, 0x02, 0x29, 0x09, 0x03, 0xCC, 0x12,
        ];
        let parsed = parse_service_data(&data).expect("valid BTHome v2 packet");
        assert_eq!(parsed.packet_id, Some(9));
        assert_eq!(parsed.battery, Some(92));
        assert_eq!(parsed.temp_centi, Some(2345));
        assert_eq!(parsed.humidity_centi, Some(4812));
        assert_eq!(parsed.voltage_mv, None);
    }

    #[test]
    fn parse_negative_temperature_and_0x45_0x2e_0x0c() {
        // Temp 0x45 (-5.4 °C => -54 = 0xFFCA LE), Humidity 0x2E (61%), Voltage 0x0C (2915 mV = 0x0B63)
        let raw_temp = (-54i16).to_le_bytes();
        let raw_volt = 2915u16.to_le_bytes();
        let data = [
            0xD2,
            0xFC,
            0x40,
            0x0C,
            raw_volt[0],
            raw_volt[1],
            0x2E,
            61,
            0x45,
            raw_temp[0],
            raw_temp[1],
        ];
        let parsed = parse_service_data(&data).expect("valid negative temp packet");
        assert_eq!(parsed.voltage_mv, Some(2915));
        assert_eq!(parsed.humidity_centi, Some(6100));
        assert_eq!(parsed.temp_centi, Some(-540));

        let mut icon_buf = [0u8; 8];
        assert_eq!(format_icon_text(parsed.temp_centi, &mut icon_buf), "-5");
    }

    #[test]
    fn ignore_encrypted_packet() {
        // Info = 0x41 (v2, encrypted bit set)
        let data = [0xD2, 0xFC, 0x41, 0x02, 0x29, 0x09];
        assert_eq!(parse_service_data(&data), None);
    }

    #[test]
    fn reject_wrong_uuid_and_wrong_version() {
        assert_eq!(parse_service_data(&[0xD3, 0xFC, 0x40, 0x02, 0x00, 0x00]), None);
        assert_eq!(parse_service_data(&[0xD2, 0xFC, 0x20, 0x02, 0x00, 0x00]), None);
        assert_eq!(parse_service_data(&[0xD2, 0xFC, 0x60, 0x02, 0x00, 0x00]), None);
        assert_eq!(parse_service_data(&[0xD2, 0xFC]), None);
    }

    #[test]
    fn handle_truncated_and_unknown_object_safely() {
        // Valid temp followed by truncated humidity (only 1 byte left for uint16)
        let truncated_after_temp = [0xD2, 0xFC, 0x40, 0x02, 0xE8, 0x03, 0x03, 0x10];
        let parsed = parse_service_data(&truncated_after_temp).expect("header valid");
        assert_eq!(parsed.temp_centi, Some(1000));
        assert_eq!(parsed.humidity_centi, None);

        // Truncated temperature immediately after Object ID
        let truncated_temp = [0xD2, 0xFC, 0x40, 0x02, 0xE8];
        let parsed2 = parse_service_data(&truncated_temp).expect("header valid");
        assert_eq!(parsed2.temp_centi, None);

        // Unknown Object ID 0x99 stops parsing cleanly without panic
        let unknown_obj = [0xD2, 0xFC, 0x40, 0x01, 80, 0x99, 0x02, 0xE8, 0x03];
        let parsed3 = parse_service_data(&unknown_obj).expect("header valid");
        assert_eq!(parsed3.battery, Some(80));
        assert_eq!(parsed3.temp_centi, None);
    }

    #[test]
    fn staleness_threshold_calculations() {
        assert_eq!(stale_threshold_secs(1), 3 * 60);
        assert_eq!(stale_threshold_secs(5), 11 * 60);
        assert_eq!(stale_threshold_secs(15), 31 * 60);
        assert_eq!(stale_threshold_secs(30), 61 * 60);
        assert_eq!(stale_threshold_secs(60), 121 * 60);

        assert!(!is_reading_stale(660, 5));
        assert!(is_reading_stale(661, 5));
        assert!(!is_reading_stale(180, 1));
        assert!(is_reading_stale(181, 1));
    }

    #[test]
    fn rounding_and_formatting_edge_cases() {
        assert_eq!(round_temp_deg(2349), 23);
        assert_eq!(round_temp_deg(2350), 24);
        assert_eq!(round_temp_deg(-49), 0);
        assert_eq!(round_temp_deg(-50), -1);
        assert_eq!(round_temp_deg(-450), -5);

        let mut icon_buf = [0u8; 8];
        assert_eq!(format_icon_text(Some(-49), &mut icon_buf), "0");
        assert_eq!(format_icon_text(Some(-50), &mut icon_buf), "-1");
        assert_eq!(format_icon_text(None, &mut icon_buf), "--");

        let reading = BthomeReading {
            battery: Some(92),
            temp_centi: Some(-54),
            humidity_centi: Some(4812),
            ..Default::default()
        };
        let mut tip = [0u16; 128];
        format_tooltip_utf16(Some(&reading), false, 5, &mut tip);
        let tip_len = tip.iter().position(|&c| c == 0).unwrap();
        let tip_str = String::from_utf16(&tip[..tip_len]).unwrap();
        assert_eq!(tip_str, "-0.5 \u{00B0}C | 48% | Bat: 92% (5m)");

        format_tooltip_utf16(Some(&reading), true, 5, &mut tip);
        let stale_len = tip.iter().position(|&c| c == 0).unwrap();
        let stale_str = String::from_utf16(&tip[..stale_len]).unwrap();
        assert_eq!(stale_str, "-0.5 \u{00B0}C | 48% | Bat: 92% (5m, stale)");
    }

    #[test]
    fn mac_parse_and_format() {
        let mac = parse_mac("a4:c1:38:12:34:56").expect("valid mac");
        assert_eq!(mac, 0x0000_A4C1_3812_3456);
        let mut buf = [0u8; 17];
        assert_eq!(format_mac(mac, &mut buf), "A4:C1:38:12:34:56");
        assert_eq!(parse_mac("A4:C1:38:12:34:ZZ"), None);
        assert_eq!(parse_mac("A4:C1:38:12:34"), None);
    }
}
