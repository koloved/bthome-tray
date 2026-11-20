//! Dynamic 32-bit ARGB tray icon renderer using Win32 GDI (`CreateDIBSection` + `CreateIconIndirect`).

use core::ffi::c_void;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, GetDC, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON, SM_CYSMICON,
};

struct Glyph<const H: usize> {
    width: i32,
    rows: [u8; H],
}

/// Bold 7x16 pixel font (2px strokes) designed to fill 16x16 and 32x32 tray icons edge-to-edge.
const fn glyph_bold_7x16(ch: u8) -> Glyph<16> {
    match ch {
        b'0' => Glyph {
            width: 7,
            rows: [
                0b0111110, 0b1111111, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011,
                0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011,
                0b1111111, 0b0111110,
            ],
        },
        b'1' => Glyph {
            width: 7,
            rows: [
                0b0001100, 0b0011100, 0b0111100, 0b1101100, 0b0001100, 0b0001100, 0b0001100,
                0b0001100, 0b0001100, 0b0001100, 0b0001100, 0b0001100, 0b0001100, 0b0001100,
                0b1111111, 0b1111111,
            ],
        },
        b'2' => Glyph {
            width: 7,
            rows: [
                0b0111110, 0b1111111, 0b1100011, 0b0000011, 0b0000011, 0b0000011, 0b0000110,
                0b0001100, 0b0011000, 0b0110000, 0b1100000, 0b1100000, 0b1100000, 0b1100000,
                0b1111111, 0b1111111,
            ],
        },
        b'3' => Glyph {
            width: 7,
            rows: [
                0b0111110, 0b1111111, 0b1100011, 0b0000011, 0b0000011, 0b0000011, 0b0000011,
                0b0011110, 0b0011110, 0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b1100011,
                0b1111111, 0b0111110,
            ],
        },
        b'4' => Glyph {
            width: 7,
            rows: [
                0b0000110, 0b0001110, 0b0011110, 0b0110110, 0b1100110, 0b1100110, 0b1100110,
                0b1100110, 0b1111111, 0b1111111, 0b0000110, 0b0000110, 0b0000110, 0b0000110,
                0b0000110, 0b0000110,
            ],
        },
        b'5' => Glyph {
            width: 7,
            rows: [
                0b1111111, 0b1111111, 0b1100000, 0b1100000, 0b1100000, 0b1100000, 0b1111110,
                0b1111111, 0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b1100011,
                0b1111111, 0b0111110,
            ],
        },
        b'6' => Glyph {
            width: 7,
            rows: [
                0b0111110, 0b1111111, 0b1100011, 0b1100000, 0b1100000, 0b1100000, 0b1111110,
                0b1111111, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011,
                0b1111111, 0b0111110,
            ],
        },
        b'7' => Glyph {
            width: 7,
            rows: [
                0b1111111, 0b1111111, 0b0000011, 0b0000011, 0b0000110, 0b0000110, 0b0001100,
                0b0001100, 0b0011000, 0b0011000, 0b0011000, 0b0011000, 0b0011000, 0b0011000,
                0b0011000, 0b0011000,
            ],
        },
        b'8' => Glyph {
            width: 7,
            rows: [
                0b0111110, 0b1111111, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011,
                0b0111110, 0b0111110, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011,
                0b1111111, 0b0111110,
            ],
        },
        b'9' => Glyph {
            width: 7,
            rows: [
                0b0111110, 0b1111111, 0b1100011, 0b1100011, 0b1100011, 0b1100011, 0b1100011,
                0b1111111, 0b0111111, 0b0000011, 0b0000011, 0b0000011, 0b0000011, 0b1100011,
                0b1111111, 0b0111110,
            ],
        },
        b'-' => Glyph {
            width: 5,
            rows: [
                0, 0, 0, 0, 0, 0, 0, 0b11111, 0b11111, 0, 0, 0, 0, 0, 0, 0,
            ],
        },
        _ => Glyph {
            width: 5,
            rows: [0; 16],
        },
    }
}

/// Compact 5x12 pixel font for 3-character strings (`-12`) and 150% DPI (`24x24` @ 2x).
const fn glyph_compact_5x12(ch: u8) -> Glyph<12> {
    match ch {
        b'0' => Glyph {
            width: 5,
            rows: [
                0b01110, 0b10001, 0b10001, 0b10011, 0b10101, 0b10101, 0b11001, 0b10001, 0b10001,
                0b10001, 0b10001, 0b01110,
            ],
        },
        b'1' => Glyph {
            width: 5,
            rows: [
                0b00100, 0b01100, 0b10100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
                0b00100, 0b00100, 0b11111,
            ],
        },
        b'2' => Glyph {
            width: 5,
            rows: [
                0b01110, 0b10001, 0b00001, 0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000,
                0b10000, 0b10000, 0b11111,
            ],
        },
        b'3' => Glyph {
            width: 5,
            rows: [
                0b01110, 0b10001, 0b00001, 0b00001, 0b00001, 0b00110, 0b00001, 0b00001, 0b00001,
                0b00001, 0b10001, 0b01110,
            ],
        },
        b'4' => Glyph {
            width: 5,
            rows: [
                0b00010, 0b00110, 0b01010, 0b10010, 0b10010, 0b10010, 0b11111, 0b00010, 0b00010,
                0b00010, 0b00010, 0b00010,
            ],
        },
        b'5' => Glyph {
            width: 5,
            rows: [
                0b11111, 0b10000, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b00001, 0b00001,
                0b00001, 0b10001, 0b01110,
            ],
        },
        b'6' => Glyph {
            width: 5,
            rows: [
                0b01110, 0b10001, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001,
                0b10001, 0b10001, 0b01110,
            ],
        },
        b'7' => Glyph {
            width: 5,
            rows: [
                0b11111, 0b00001, 0b00001, 0b00010, 0b00010, 0b00100, 0b00100, 0b01000, 0b01000,
                0b01000, 0b01000, 0b01000,
            ],
        },
        b'8' => Glyph {
            width: 5,
            rows: [
                0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b10001,
                0b10001, 0b10001, 0b01110,
            ],
        },
        b'9' => Glyph {
            width: 5,
            rows: [
                0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b00001,
                0b00001, 0b10001, 0b01110,
            ],
        },
        b'-' => Glyph {
            width: 3,
            rows: [0, 0, 0, 0, 0, 0b111, 0b111, 0, 0, 0, 0, 0],
        },
        _ => Glyph {
            width: 3,
            rows: [0; 12],
        },
    }
}

struct GdiBitmap(HBITMAP);

impl Drop for GdiBitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0.0));
            }
        }
    }
}

/// Returns the system small-icon dimensions `(width, height)` for the current DPI.
pub fn small_icon_size() -> (i32, i32) {
    let w = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let h = unsafe { GetSystemMetrics(SM_CYSMICON) }.clamp(16, 64);
    (w, h)
}

fn measure_bold(bytes: &[u8], gap: i32) -> i32 {
    let mut w = 0i32;
    for (idx, &ch) in bytes.iter().enumerate() {
        if idx > 0 {
            w += gap;
        }
        w += glyph_bold_7x16(ch).width;
    }
    w
}

fn measure_compact(bytes: &[u8], gap: i32) -> i32 {
    let mut w = 0i32;
    for (idx, &ch) in bytes.iter().enumerate() {
        if idx > 0 {
            w += gap;
        }
        w += glyph_compact_5x12(ch).width;
    }
    w
}

struct Canvas<'a> {
    pixels: &'a mut [u32],
    w: i32,
    h: i32,
    fg_argb: u32,
}

impl<'a> Canvas<'a> {
    fn blit_glyph<const H: usize>(
        &mut self,
        g: Glyph<H>,
        cursor_x: i32,
        start_y: i32,
        scale: i32,
    ) {
        for (row_idx, &row_bits) in g.rows.iter().enumerate() {
            for col in 0..g.width {
                let bit_pos = g.width - 1 - col;
                if (row_bits >> bit_pos) & 1 != 0 {
                    for sy in 0..scale {
                        let py = start_y + (row_idx as i32) * scale + sy;
                        if py < 0 || py >= self.h {
                            continue;
                        }
                        for sx in 0..scale {
                            let px = cursor_x + col * scale + sx;
                            if px >= 0 && px < self.w {
                                self.pixels[(py as usize) * (self.w as usize) + (px as usize)] =
                                    self.fg_argb;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Renders `text` (e.g. `"24"`, `"-5"`, `"--"`) into a 32-bit ARGB `HICON`.
///
/// The caller owns the returned `HICON` and must destroy it with `DestroyIcon`.
pub fn render_temp_icon(text: &str, light_taskbar: bool) -> Option<HICON> {
    let (w, h) = small_icon_size();
    let fg_argb: u32 = if light_taskbar { 0xFF14_1414 } else { 0xFFFF_FFFF };

    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let bold_gap = 2;
    let bold_base_w = measure_bold(bytes, bold_gap);
    let bold_scale = if bold_base_w <= w && 16 <= h {
        (w / bold_base_w).min(h / 16)
    } else {
        0
    };

    let compact_gap = 1;
    let compact_base_w = measure_compact(bytes, compact_gap);
    let compact_scale = if compact_base_w <= w && 12 <= h {
        (w / compact_base_w).min(h / 12)
    } else {
        0
    };

    let use_bold = bold_scale >= 1 && (bold_scale * 16 >= compact_scale * 12);

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            // Negative height creates a top-down DIB so row 0 is the top edge.
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let hdc = unsafe { GetDC(None) };
    let mut bits_ptr: *mut c_void = core::ptr::null_mut();
    let color_bmp_res = unsafe {
        CreateDIBSection(
            Some(hdc),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            None,
            0,
        )
    };
    unsafe {
        let _ = ReleaseDC(None, hdc);
    }

    let color_bmp = color_bmp_res.ok()?;
    let _color_guard = GdiBitmap(color_bmp);
    if bits_ptr.is_null() {
        return None;
    }

    let pixel_count = (w as usize) * (h as usize);
    let pixels = unsafe { core::slice::from_raw_parts_mut(bits_ptr as *mut u32, pixel_count) };
    pixels.fill(0x0000_0000);
    let mut canvas = Canvas {
        pixels,
        w,
        h,
        fg_argb,
    };

    if use_bold {
        let draw_w = bold_base_w * bold_scale;
        let draw_h = 16 * bold_scale;
        let mut cursor_x = (w - draw_w) / 2;
        let start_y = (h - draw_h) / 2;
        for (idx, &ch) in bytes.iter().enumerate() {
            if idx > 0 {
                cursor_x += bold_gap * bold_scale;
            }
            let g = glyph_bold_7x16(ch);
            let gw = g.width;
            canvas.blit_glyph(g, cursor_x, start_y, bold_scale);
            cursor_x += gw * bold_scale;
        }
    } else {
        let scale = compact_scale.max(1);
        let draw_w = compact_base_w * scale;
        let draw_h = 12 * scale;
        let mut cursor_x = (w - draw_w) / 2;
        let start_y = (h - draw_h) / 2;
        for (idx, &ch) in bytes.iter().enumerate() {
            if idx > 0 {
                cursor_x += compact_gap * scale;
            }
            let g = glyph_compact_5x12(ch);
            let gw = g.width;
            canvas.blit_glyph(g, cursor_x, start_y, scale);
            cursor_x += gw * scale;
        }
    }

    // 1bpp monochrome AND mask (all zeros; alpha channel in hbmColor controls transparency).
    let mask_stride = (((w + 15) / 16) * 2) as usize;
    let mask_bytes = mask_stride * (h as usize);
    let mask_buf = [0u8; 512];
    let mask_ptr = if mask_bytes <= mask_buf.len() {
        Some(mask_buf.as_ptr() as *const c_void)
    } else {
        None
    };
    let mask_bmp = unsafe { CreateBitmap(w, h, 1, 1, mask_ptr) };
    if mask_bmp.is_invalid() {
        return None;
    }
    let _mask_guard = GdiBitmap(mask_bmp);

    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask_bmp,
        hbmColor: color_bmp,
    };

    unsafe { CreateIconIndirect(&icon_info) }.ok()
}

pub fn destroy_icon(icon: HICON) {
    if !icon.is_invalid() {
        unsafe {
            let _ = DestroyIcon(icon);
        }
    }
}
