//! The bits of Windows 11 the UI follows: light/dark app mode and the accent
//! palette (both from the user's Personalization settings), the Mica system
//! backdrop behind the window, the "Animation effects" accessibility switch,
//! and the Segoe Fluent Icons glyphs, read from the installed system font.
//!
//! Everything degrades quietly: off Windows, or on Windows 10, the theme falls
//! back to the default blue accent, a solid background and the bundled icons.

use std::sync::OnceLock;

/// Light/dark mode and the accent palette from the Windows settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemTheme {
    pub dark: bool,
    /// The accent ramp as `0xRRGGBB`: `[light3, light2, light1, base, dark1, dark2, dark3]`.
    pub accent: [u32; 7],
}

/// Windows' default blue, used when the palette can't be read.
const DEFAULT_ACCENT: [u32; 7] = [0x99ebff, 0x4cc2ff, 0x0091f8, 0x0078d4, 0x005a9e, 0x003e92, 0x001a68];

impl Default for SystemTheme {
    fn default() -> Self {
        Self { dark: true, accent: DEFAULT_ACCENT }
    }
}

/// Reads the current app mode and accent palette.
pub fn system_theme() -> SystemTheme {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let dark = hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
            .map(|light| light == 0)
            .unwrap_or(true);
        let accent = hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
            .and_then(|k| k.get_raw_value("AccentPalette"))
            .ok()
            .and_then(|v| parse_accent_palette(&v.bytes))
            .unwrap_or(DEFAULT_ACCENT);
        // For checking both themes without changing Windows settings.
        let theme = std::env::var("VAULTER_THEME").or_else(|_| std::env::var("VAULT_PATCHER_THEME"));
        let dark = match theme.as_deref() {
            Ok("light") => false,
            Ok("dark") => true,
            _ => dark,
        };
        // For demos and screenshots: a brand accent without touching Windows.
        let accent = std::env::var("VAULTER_ACCENT").ok().and_then(|hex| accent_palette_from(&hex)).unwrap_or(accent);
        SystemTheme { dark, accent }
    }
    #[cfg(not(windows))]
    SystemTheme::default()
}

/// `AccentPalette` is eight RGBA quads, lightest first; the eighth is unused.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_accent_palette(bytes: &[u8]) -> Option<[u32; 7]> {
    if bytes.len() < 28 {
        return None;
    }
    let mut out = [0u32; 7];
    for (i, c) in bytes.as_chunks::<4>().0.iter().take(7).enumerate() {
        out[i] = (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
    }
    Some(out)
}

/// An accent palette (Light3..Dark3) where `#RRGGBB` is the fill of accent
/// buttons in both themes (Windows would use a lighter shade in dark mode).
/// A deep color keeps white text readable on it; the gradient ends and
/// accent text get nearby shades.
#[cfg_attr(not(windows), allow(dead_code))]
fn accent_palette_from(hex: &str) -> Option<[u32; 7]> {
    let v = u32::from_str_radix(hex.trim().trim_start_matches('#'), 16).ok()?;
    let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    let mix = |to: u32, t: f32| {
        let c = |x: u32| (x as f32 + (to as f32 - x as f32) * t).round() as u32;
        (c(r) << 16) | (c(g) << 8) | c(b)
    };
    let fill = v & 0xff_ffff;
    // Dark mode fills with [1] (gradient to [2]); light mode with [4] (from [3]).
    Some([mix(255, 0.5), fill, mix(0, 0.12), fill, mix(0, 0.12), mix(0, 0.3), mix(0, 0.45)])
}

/// Whether the "Animation effects" setting (Accessibility › Visual effects) is on.
pub fn animations_enabled() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
        let mut on: i32 = 1;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to the pointer.
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) };
        ok == 0 || on != 0
    }
    #[cfg(not(windows))]
    true
}

/// Hands the process's unused memory back to Windows (its working set is
/// trimmed; pages come back on demand). Used when Vaulter goes into
/// background mode, so a running game has that RAM.
pub fn trim_memory() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        // SAFETY: (-1, -1) is the documented "trim now" request for our own process.
        unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
    }
}

/// The main display's refresh rate, for the frame-rate setting's warning.
pub fn display_refresh_hz() -> Option<u32> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
        // SAFETY: DEVMODEW is plain data; dmSize tells the API its size.
        let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        let ok = unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode) };
        (ok != 0 && mode.dmDisplayFrequency > 1).then_some(mode.dmDisplayFrequency)
    }
    #[cfg(not(windows))]
    None
}

/// Puts the Mica backdrop behind the window, with rounded corners. Returns
/// false where the system can't (Windows 10, or 11 before 22H2) so the
/// theme paints a solid background instead.
pub fn apply_mica(window: &gpui::Window) -> bool {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::Graphics::Dwm::{
            DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
        };
        use windows_sys::Win32::UI::Controls::MARGINS;

        let Ok(handle) = HasWindowHandle::window_handle(window) else { return false };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return false };
        let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        let backdrop = DWMSBT_MAINWINDOW;
        let corners = DWMWCP_ROUND;
        // Mica's tint follows the frame's dark mode; match our theme (which
        // VAULTER_THEME can override) rather than the system's.
        let dark = i32::from(system_theme().dark);
        // The backdrop shows through wherever the client area is transparent;
        // gpui renders through DirectComposition with premultiplied alpha.
        let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        // SAFETY: plain DWM calls on our own top-level window with correctly
        // sized attribute values.
        unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as _, (&corners as *const i32).cast(), 4);
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as _, (&dark as *const i32).cast(), 4);
            if DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE as _, (&backdrop as *const i32).cast(), 4) < 0 {
                return false;
            }
            DwmExtendFrameIntoClientArea(hwnd, &margins) >= 0
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}

/// The installed Segoe Fluent Icons font (Windows 11) or Segoe MDL2 Assets
/// (Windows 10), loaded once. Never bundled: it's read from the system.
fn icon_font() -> Option<&'static [u8]> {
    static FONT: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    FONT.get_or_init(|| {
        let fonts = std::env::var_os("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts"))?;
        ["SegoeIcons.ttf", "segmdl2.ttf"].iter().find_map(|f| std::fs::read(fonts.join(f)).ok())
    })
    .as_deref()
}

/// One glyph of the system icon font as a standalone SVG (the font's 2048
/// em square is the icon canvas), or None when the font or glyph is missing.
pub fn glyph_svg(codepoint: char) -> Option<Vec<u8>> {
    // Built once per glyph: the atlas asks again for every new size.
    use std::collections::HashMap;
    use std::sync::Mutex;
    static MEMO: Mutex<Option<HashMap<char, Option<Vec<u8>>>>> = Mutex::new(None);
    let mut memo = MEMO.lock().unwrap_or_else(|e| e.into_inner());
    memo.get_or_insert_with(HashMap::new).entry(codepoint).or_insert_with(|| build_glyph_svg(codepoint)).clone()
}

fn build_glyph_svg(codepoint: char) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(icon_font()?, 0).ok()?;
    let glyph = face.glyph_index(codepoint)?;
    let em = f32::from(face.units_per_em());
    let mut path = SvgPath(String::new());
    face.outline_glyph(glyph, &mut path)?;
    Some(
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {em} {em}"><path transform="matrix(1 0 0 -1 0 {asc})" d="{}"/></svg>"#,
            path.0,
            asc = face.ascender(),
        )
        .into_bytes(),
    )
}

struct SvgPath(String);

impl ttf_parser::OutlineBuilder for SvgPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push_str(&format!("M{x} {y}"));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push_str(&format!("L{x} {y}"));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.push_str(&format!("Q{x1} {y1} {x} {y}"));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.push_str(&format!("C{x1} {y1} {x2} {y2} {x} {y}"));
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_palette_is_read_lightest_first() {
        let bytes = [
            0x99, 0xeb, 0xff, 0, 0x4c, 0xc2, 0xff, 0, 0x00, 0x91, 0xf8, 0, 0x00, 0x78, 0xd4, 0, 0x00, 0x5a, 0x9e, 0, 0x00,
            0x3e, 0x92, 0, 0x00, 0x1a, 0x68, 0, 0xf7, 0x63, 0x0c, 0,
        ];
        assert_eq!(parse_accent_palette(&bytes), Some(DEFAULT_ACCENT));
        assert_eq!(parse_accent_palette(&bytes[..20]), None);
    }

    #[test]
    #[cfg(windows)]
    fn system_icon_font_glyphs_become_svg() {
        // Every Windows 10/11 install has one of the two icon fonts.
        let svg = String::from_utf8(glyph_svg('\u{E8BB}').expect("ChromeClose glyph")).unwrap();
        assert!(svg.starts_with("<svg") && svg.contains("d=\"M"));
    }
}
