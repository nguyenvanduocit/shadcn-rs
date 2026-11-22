//! The user's text rendering settings, read the way Chrome reads them on each OS.
//!
//! The text rasterizer uses them to choose grayscale or subpixel (LCD) coverage, the LCD stripe
//! order, the Windows ClearType gamma and contrast overrides, and macOS stem darkening. They are
//! re-read when a window regains focus, so every read is cheap and bounded in time: calls that
//! talk to another process (D-Bus, the X server) run on a helper thread with a deadline, and any
//! error falls back to the OS default.
//!
//! The parsers and the mappings from raw values to [`TextSettings`] are plain functions compiled
//! on every OS, so they are tested without the OS that produces their input.

use std::time::Duration;

/// The order of the colour stripes inside one LCD pixel, as subpixel coverage needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subpixel {
    /// Red, green, blue from left to right.
    Rgb,
    /// Blue, green, red from left to right.
    Bgr,
    /// Red, green, blue from top to bottom.
    VRgb,
    /// Blue, green, red from top to bottom.
    VBgr,
}

/// How the user asked the OS to draw text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSettings {
    /// Glyphs are antialiased at all. False means aliased (monochrome) text was requested; it is
    /// recorded so the renderer can decide, it may still antialias.
    pub antialias: bool,
    /// Subpixel (LCD) coverage is on, with this stripe order; `None` means grayscale.
    pub subpixel: Option<Subpixel>,
    /// Windows: the ClearType gamma the user tuned, when they tuned it; `None` keeps the
    /// renderer's default (Chrome then uses Skia's default as well).
    pub gamma: Option<f32>,
    /// Windows: the ClearType enhanced contrast the user tuned, when they tuned it; `None` keeps
    /// the renderer's default.
    pub contrast: Option<f32>,
    /// macOS: font smoothing is on, so CoreText thickens strokes and so do we. On by default
    /// when the user never changed it.
    pub stem_darkening: bool,
}

impl TextSettings {
    /// What this OS does when the user never changed anything.
    ///
    /// Windows ships ClearType on with an RGB stripe; macOS has smoothed grayscale text; Linux
    /// and the BSDs, with no desktop settings to read, get grayscale antialiasing as Chrome does.
    #[must_use]
    pub fn default_for_os() -> Self {
        let gray = Self {
            antialias: true,
            subpixel: None,
            gamma: None,
            contrast: None,
            stem_darkening: false,
        };
        if cfg!(windows) {
            Self {
                subpixel: Some(Subpixel::Rgb),
                ..gray
            }
        } else if cfg!(target_os = "macos") {
            Self {
                stem_darkening: true,
                ..gray
            }
        } else {
            gray
        }
    }
}

/// The current OS's settings, or its defaults when they cannot be read.
#[must_use]
pub fn read() -> TextSettings {
    platform::read()
}

/// The environment variable that replaces the OS settings, for tests and golden images.
pub const OVERRIDE_VAR: &str = "GUI_TEXT_AA";

/// Like [`read`], except that `GUI_TEXT_AA` set to a known value replaces the OS settings.
///
/// The accepted values are `gray`, `lcd-rgb`, `lcd-bgr`, `lcd-vrgb`, `lcd-vbgr` and `none`. With
/// an override the OS is not read at all: the result is this OS's defaults with antialiasing
/// and stripe order replaced, so golden images do not depend on the machine's settings. An
/// unknown value is ignored.
#[must_use]
pub fn read_with_override() -> TextSettings {
    std::env::var(OVERRIDE_VAR)
        .ok()
        .and_then(|value| parse_override(&value, TextSettings::default_for_os()))
        .unwrap_or_else(read)
}

/// Applies a `GUI_TEXT_AA` value to `base`; `None` when the value is not one of the known ones.
#[must_use]
pub fn parse_override(value: &str, base: TextSettings) -> Option<TextSettings> {
    let (antialias, subpixel) = match value.trim().to_ascii_lowercase().as_str() {
        "none" => (false, None),
        "gray" | "grey" | "grayscale" => (true, None),
        "lcd-rgb" => (true, Some(Subpixel::Rgb)),
        "lcd-bgr" => (true, Some(Subpixel::Bgr)),
        "lcd-vrgb" => (true, Some(Subpixel::VRgb)),
        "lcd-vbgr" => (true, Some(Subpixel::VBgr)),
        _ => return None,
    };
    Some(TextSettings {
        antialias,
        subpixel,
        ..base
    })
}

/// Settings with neither gamma, contrast nor stem darkening, the shape every non-Apple,
/// non-Windows source produces.
fn plain(antialias: bool, subpixel: Option<Subpixel>) -> TextSettings {
    TextSettings {
        antialias,
        subpixel: if antialias { subpixel } else { None },
        gamma: None,
        contrast: None,
        stem_darkening: false,
    }
}

/// The stripe order named the way fontconfig, Xft and GNOME name it; `None` for `none`,
/// `unknown` or anything else.
#[must_use]
pub fn parse_rgba(value: &str) -> Option<Subpixel> {
    match value.trim() {
        "rgb" => Some(Subpixel::Rgb),
        "bgr" => Some(Subpixel::Bgr),
        "vrgb" => Some(Subpixel::VRgb),
        "vbgr" => Some(Subpixel::VBgr),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// XSETTINGS (X11 desktops: gnome-settings-daemon, xsettingsd, xfsettingsd, ...)
// ---------------------------------------------------------------------------------------------

/// One XSETTINGS value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XValue {
    /// A signed 32-bit integer.
    Int(i32),
    /// A UTF-8 string (invalid bytes are replaced).
    Str(String),
    /// Red, green, blue and alpha, 16 bits each.
    Color([u16; 4]),
}

/// One named XSETTINGS entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XSetting {
    /// The setting's name, such as `Xft/RGBA`.
    pub name: String,
    /// The settings-manager serial at which this entry last changed.
    pub serial: u32,
    /// The value.
    pub value: XValue,
}

/// The decoded `_XSETTINGS_SETTINGS` property.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct XSettings {
    /// The serial of the whole set, bumped by the manager on every change.
    pub serial: u32,
    /// The entries in wire order.
    pub settings: Vec<XSetting>,
}

impl XSettings {
    /// The value named `name`, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&XValue> {
        self.settings
            .iter()
            .find(|setting| setting.name == name)
            .map(|setting| &setting.value)
    }

    /// The text settings these XSETTINGS describe, or `None` when they say nothing about
    /// antialiasing, so another source may be asked.
    #[must_use]
    pub fn text_settings(&self) -> Option<TextSettings> {
        let antialias = match self.get("Xft/Antialias") {
            Some(XValue::Int(value)) => Some(*value),
            _ => None,
        };
        let rgba = match self.get("Xft/RGBA") {
            Some(XValue::Str(value)) => Some(value.as_str()),
            _ => None,
        };
        if antialias.is_none() && rgba.is_none() {
            return None;
        }
        Some(from_xft(antialias, rgba))
    }
}

/// Maps `Xft/Antialias` (1 on, 0 off, -1 or absent: the default, on) and `Xft/RGBA` (`rgb`,
/// `bgr`, `vrgb`, `vbgr`; `none` or absent: grayscale) as GTK hands them to Chrome.
#[must_use]
pub fn from_xft(antialias: Option<i32>, rgba: Option<&str>) -> TextSettings {
    plain(antialias != Some(0), rgba.and_then(parse_rgba))
}

/// A cursor over the XSETTINGS wire format in its declared byte order.
struct XReader<'a> {
    bytes: &'a [u8],
    at: usize,
    big_endian: bool,
}

impl<'a> XReader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(len)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn skip_padding(&mut self, len: usize) -> Option<()> {
        let pad = len.wrapping_neg() & 3;
        self.take(pad).map(|_| ())
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }

    fn u16(&mut self) -> Option<u16> {
        let bytes: [u8; 2] = self.take(2)?.try_into().ok()?;
        Some(if self.big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    }

    fn i32(&mut self) -> Option<i32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(if self.big_endian {
            i32::from_be_bytes(bytes)
        } else {
            i32::from_le_bytes(bytes)
        })
    }

    /// A length-prefixed byte string padded to four bytes.
    fn padded(&mut self, len: usize) -> Option<String> {
        let bytes = self.take(len)?;
        self.skip_padding(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Decodes an `_XSETTINGS_SETTINGS` property as the XSETTINGS specification lays it out: a byte
/// order byte (0 little-endian, 1 big-endian), three unused bytes, the serial, the entry count,
/// then each entry (type, unused byte, name length, name padded to four bytes, last-change
/// serial, and an integer, a length-prefixed padded string, or four 16-bit colour channels).
///
/// Returns `None` when the blob is malformed. An entry of an unknown type ends the parse, since
/// its size is unknown; the entries before it are kept.
#[must_use]
pub fn parse_xsettings(bytes: &[u8]) -> Option<XSettings> {
    let big_endian = match bytes.first()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let mut reader = XReader {
        bytes,
        at: 4,
        big_endian,
    };
    let serial = reader.u32()?;
    let count = reader.u32()?;
    let mut settings = Vec::new();
    for _ in 0..count {
        let kind = reader.u8()?;
        reader.take(1)?;
        let name_len = usize::from(reader.u16()?);
        let name = reader.padded(name_len)?;
        let last_change = reader.u32()?;
        let value = match kind {
            0 => XValue::Int(reader.i32()?),
            1 => {
                let len = usize::try_from(reader.u32()?).ok()?;
                XValue::Str(reader.padded(len)?)
            }
            2 => XValue::Color([reader.u16()?, reader.u16()?, reader.u16()?, reader.u16()?]),
            _ => break,
        };
        settings.push(XSetting {
            name,
            serial: last_change,
            value,
        });
    }
    Some(XSettings { serial, settings })
}

// ---------------------------------------------------------------------------------------------
// GNOME settings, as the XDG desktop portal exposes them on Wayland
// ---------------------------------------------------------------------------------------------

/// Maps GNOME's `font-antialiasing` (`grayscale`, `rgba`, `none`) and `font-rgba-order` (`rgb`,
/// `bgr`, `vrgb`, `vbgr`) from `org.gnome.desktop.interface`, as gnome-settings-daemon turns them
/// into `Xft/Antialias` and `Xft/RGBA`. Subpixel order without `rgba` antialiasing is grayscale;
/// `rgba` with no or an unknown order uses RGB, GNOME's default order.
#[must_use]
pub fn from_gnome(antialiasing: Option<&str>, rgba_order: Option<&str>) -> TextSettings {
    match antialiasing.map(str::trim) {
        Some("none") => plain(false, None),
        Some("rgba") => plain(true, Some(rgba_order.and_then(parse_rgba).unwrap_or(Subpixel::Rgb))),
        _ => plain(true, None),
    }
}

// ---------------------------------------------------------------------------------------------
// Windows registry
// ---------------------------------------------------------------------------------------------

/// The registry values Chrome's Windows text settings come from.
///
/// `HKEY_CURRENT_USER\Control Panel\Desktop` backs `SystemParametersInfo`: `FontSmoothing`
/// (`"2"` on, `"0"` off), `FontSmoothingType` (1 standard grayscale, 2 ClearType),
/// `FontSmoothingOrientation` (1 RGB, 0 BGR) and `FontSmoothingGamma` (1000 to 2200).
/// `HKEY_CURRENT_USER\Software\Microsoft\Avalon.Graphics\DISPLAYn` is written by the ClearType
/// Text Tuner and read by DirectWrite: `PixelStructure` (0 flat, 1 RGB, 2 BGR), `GammaLevel`
/// (1000 to 2200, thousandths) and `EnhancedContrastLevel` (0 to 1000, hundredths).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WindowsRegistry {
    /// `FontSmoothing`.
    pub font_smoothing: Option<u32>,
    /// `FontSmoothingType`.
    pub smoothing_type: Option<u32>,
    /// `FontSmoothingOrientation`.
    pub orientation: Option<u32>,
    /// `FontSmoothingGamma`.
    pub smoothing_gamma: Option<u32>,
    /// The tuner's key for the primary display, when it exists.
    pub tuner: Option<ClearTypeTuner>,
}

/// The ClearType Text Tuner's values for one display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ClearTypeTuner {
    /// `PixelStructure`.
    pub pixel_structure: Option<u32>,
    /// `GammaLevel`.
    pub gamma_level: Option<u32>,
    /// `EnhancedContrastLevel`.
    pub enhanced_contrast_level: Option<u32>,
}

/// Skia's text contrast range (`SkSurfaceProps::kMinContrastInclusive..=kMaxContrastInclusive`),
/// to which Chrome clamps DirectWrite's value.
const CONTRAST_RANGE: (f32, f32) = (0.0, 1.0);
/// Skia's text gamma range. Its top end is exclusive; Chrome clamps to `4 - f32::EPSILON`, which
/// rounds back to 4, so we clamp to the largest float below 4 instead.
const GAMMA_RANGE: (f32, f32) = (0.0, 4.0_f32.next_down());

/// Converts a registry level to a float in `1 / divisor` units, clamped to `range`.
fn level(value: u32, divisor: u16, range: (f32, f32)) -> Option<f32> {
    let value = u16::try_from(value).unwrap_or(u16::MAX);
    Some((f32::from(value) / f32::from(divisor)).clamp(range.0, range.1))
}

/// Maps the Windows registry values the way Chrome's `font_render_params_win.cc` does.
///
/// Smoothing and its type come from `SystemParametersInfo`; absent values mean the Windows
/// default (smoothing on, ClearType). The stripe order comes from the tuner's `PixelStructure`
/// when the tuner key exists (0 is flat, so grayscale), else from `FontSmoothingOrientation`.
/// Gamma and contrast are reported only when the tuner key exists, as Chrome only applies
/// DirectWrite's values then; DirectWrite's gamma is the tuner's `GammaLevel`, else the system's
/// `FontSmoothingGamma` when it is in its valid 1000 to 2200 range. Both are clamped to Skia's
/// ranges as Chrome clamps them.
#[must_use]
pub fn from_windows(registry: &WindowsRegistry) -> TextSettings {
    let smoothing = registry.font_smoothing.is_none_or(|value| value != 0);
    let cleartype = registry.smoothing_type.is_none_or(|value| value == 2);
    let tuner = registry.tuner;
    let subpixel = if !smoothing || !cleartype {
        None
    } else if let Some(structure) = tuner.and_then(|tuner| tuner.pixel_structure) {
        match structure {
            1 => Some(Subpixel::Rgb),
            2 => Some(Subpixel::Bgr),
            _ => None,
        }
    } else {
        match registry.orientation {
            Some(0) => Some(Subpixel::Bgr),
            _ => Some(Subpixel::Rgb),
        }
    };
    let (gamma, contrast) = match tuner {
        Some(tuner) => {
            let system_gamma = registry.smoothing_gamma.filter(|value| (1000..=2200).contains(value));
            let gamma = tuner
                .gamma_level
                .or(system_gamma)
                .and_then(|value| level(value, 1000, GAMMA_RANGE));
            let contrast = tuner
                .enhanced_contrast_level
                .and_then(|value| level(value, 100, CONTRAST_RANGE));
            (gamma, contrast)
        }
        None => (None, None),
    };
    TextSettings {
        antialias: smoothing,
        subpixel,
        gamma,
        contrast,
        stem_darkening: false,
    }
}

/// Reads a registry value written as a number in a string (`"2"`), as `FontSmoothing` is.
#[must_use]
pub fn parse_registry_number(text: &str) -> Option<u32> {
    text.trim_matches(char::from(0)).trim().parse().ok()
}

// ---------------------------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------------------------

/// Maps `AppleFontSmoothing` from the global preferences: 0 turns CoreText's stroke thickening
/// off, any other value or no value leaves it on. macOS never draws LCD text any more, so
/// coverage is always grayscale.
#[must_use]
pub fn from_macos(apple_font_smoothing: Option<i64>) -> TextSettings {
    TextSettings {
        antialias: true,
        subpixel: None,
        gamma: None,
        contrast: None,
        stem_darkening: apple_font_smoothing != Some(0),
    }
}

/// Runs `work` on a helper thread and waits at most `limit` for it, so a hung D-Bus daemon or X
/// server cannot stall the UI thread; the helper thread is left to finish on its own.
#[cfg_attr(
    not(any(test, all(unix, not(target_os = "macos"), not(target_os = "android")))),
    expect(dead_code, reason = "only the Linux and BSD readers talk to other processes")
)]
fn with_deadline<T: Send + 'static>(limit: Duration, work: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("text-settings".into())
        .spawn(move || {
            // The receiver is gone when the deadline passed; nothing is left to tell.
            sender.send(work()).ok();
        })
        .ok()?;
    receiver.recv_timeout(limit).ok().flatten()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{TextSettings, from_macos};

    /// Reads `AppleFontSmoothing` from the user's global preferences file. The preferences
    /// daemon writes the file shortly after a change, which is soon enough for a re-read on
    /// focus.
    pub(super) fn read() -> TextSettings {
        let Some(home) = std::env::var_os("HOME") else {
            return TextSettings::default_for_os();
        };
        let path = std::path::Path::new(&home).join("Library/Preferences/.GlobalPreferences.plist");
        let Ok(value) = plist::Value::from_file(path) else {
            return TextSettings::default_for_os();
        };
        let smoothing = value
            .as_dictionary()
            .and_then(|dict| dict.get("AppleFontSmoothing"))
            .and_then(|value| {
                value
                    .as_signed_integer()
                    .or_else(|| value.as_boolean().map(i64::from))
                    .or_else(|| value.as_string().and_then(|text| text.trim().parse().ok()))
            });
        from_macos(smoothing)
    }
}

#[cfg(windows)]
mod platform {
    use super::{ClearTypeTuner, TextSettings, WindowsRegistry, from_windows, parse_registry_number};
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    /// Reads a value stored either as a DWORD or as a decimal string.
    fn number(key: &RegKey, name: &str) -> Option<u32> {
        key.get_value::<u32, _>(name).ok().or_else(|| {
            key.get_value::<String, _>(name)
                .ok()
                .and_then(|text| parse_registry_number(&text))
        })
    }

    /// Opens the tuner key of the primary display. Chrome names it from `EnumDisplayDevices`,
    /// which has no safe binding here; the primary display is `DISPLAY1` on almost every
    /// machine, so that is preferred, then the first `DISPLAY` key in name order.
    fn tuner_key(user: &RegKey) -> Option<RegKey> {
        let avalon = user.open_subkey("Software\\Microsoft\\Avalon.Graphics").ok()?;
        if let Ok(key) = avalon.open_subkey("DISPLAY1") {
            return Some(key);
        }
        let mut names: Vec<String> = avalon
            .enum_keys()
            .filter_map(Result::ok)
            .filter(|name| name.to_ascii_uppercase().starts_with("DISPLAY"))
            .collect();
        names.sort();
        names.first().and_then(|name| avalon.open_subkey(name).ok())
    }

    pub(super) fn read() -> TextSettings {
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let mut registry = WindowsRegistry::default();
        if let Ok(desktop) = user.open_subkey("Control Panel\\Desktop") {
            registry.font_smoothing = number(&desktop, "FontSmoothing");
            registry.smoothing_type = number(&desktop, "FontSmoothingType");
            registry.orientation = number(&desktop, "FontSmoothingOrientation");
            registry.smoothing_gamma = number(&desktop, "FontSmoothingGamma");
        }
        registry.tuner = tuner_key(&user).map(|key| ClearTypeTuner {
            pixel_structure: number(&key, "PixelStructure"),
            gamma_level: number(&key, "GammaLevel"),
            enhanced_contrast_level: number(&key, "EnhancedContrastLevel"),
        });
        from_windows(&registry)
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod platform {
    use super::{TextSettings, parse_xsettings, with_deadline};
    use std::time::Duration;

    /// How long one source may take before it counts as not answering. Both answer in well
    /// under a millisecond when healthy.
    const DEADLINE: Duration = Duration::from_millis(200);

    /// On Wayland the portal speaks for the desktop and XSETTINGS (through Xwayland) is a
    /// fallback; on X11 the order is reversed. Grayscale when neither answers.
    pub(super) fn read() -> TextSettings {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
        let found = if wayland {
            portal().or_else(xsettings)
        } else {
            xsettings().or_else(portal)
        };
        found.unwrap_or_else(TextSettings::default_for_os)
    }

    fn xsettings() -> Option<TextSettings> {
        if std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty()) {
            return None;
        }
        with_deadline(DEADLINE, || {
            x11::property().and_then(|bytes| parse_xsettings(&bytes)?.text_settings())
        })
    }

    fn portal() -> Option<TextSettings> {
        with_deadline(DEADLINE, portal::text_settings)
    }

    mod x11 {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

        /// The `_XSETTINGS_SETTINGS` property of the settings manager of the default screen.
        pub(super) fn property() -> Option<Vec<u8>> {
            let (conn, screen) = x11rb::connect(None).ok()?;
            let selection = format!("_XSETTINGS_S{screen}");
            let selection = conn.intern_atom(true, selection.as_bytes()).ok()?.reply().ok()?.atom;
            if selection == u32::from(AtomEnum::NONE) {
                return None;
            }
            let owner = conn.get_selection_owner(selection).ok()?.reply().ok()?.owner;
            if owner == x11rb::NONE {
                return None;
            }
            let settings = conn.intern_atom(true, b"_XSETTINGS_SETTINGS").ok()?.reply().ok()?.atom;
            if settings == u32::from(AtomEnum::NONE) {
                return None;
            }
            // The length is in 32-bit units; a quarter of the address space is the whole property.
            let reply = conn
                .get_property(false, owner, settings, AtomEnum::ANY, 0, u32::MAX / 4)
                .ok()?
                .reply()
                .ok()?;
            (reply.format == 8).then_some(reply.value)
        }
    }

    mod portal {
        use super::super::{TextSettings, from_gnome};
        use super::DEADLINE;
        use zbus::blocking::Connection;
        use zbus::blocking::connection::Builder;
        use zbus::zvariant::{OwnedValue, Value};

        const NAMESPACE: &str = "org.gnome.desktop.interface";

        /// GNOME's font settings through `org.freedesktop.portal.Settings`; `None` when the
        /// portal is missing or exposes neither key.
        pub(super) fn text_settings() -> Option<TextSettings> {
            let conn = Builder::session().ok()?.method_timeout(DEADLINE).build().ok()?;
            let antialiasing = read_string(&conn, "font-antialiasing");
            let order = read_string(&conn, "font-rgba-order");
            if antialiasing.is_none() && order.is_none() {
                return None;
            }
            Some(from_gnome(antialiasing.as_deref(), order.as_deref()))
        }

        /// `ReadOne` (portal version 2) returns the value itself; the older `Read` wraps it in
        /// one more variant, which [`string`] unwraps.
        fn read_string(conn: &Connection, key: &str) -> Option<String> {
            ["ReadOne", "Read"].into_iter().find_map(|method| {
                let reply = conn
                    .call_method(
                        Some("org.freedesktop.portal.Desktop"),
                        "/org/freedesktop/portal/desktop",
                        Some("org.freedesktop.portal.Settings"),
                        method,
                        &(NAMESPACE, key),
                    )
                    .ok()?;
                let value: OwnedValue = reply.body().deserialize().ok()?;
                string(&value)
            })
        }

        fn string(value: &Value<'_>) -> Option<String> {
            match value {
                Value::Str(text) => Some(text.as_str().to_owned()),
                Value::Value(inner) => string(inner),
                _ => None,
            }
        }
    }
}

#[cfg(not(any(
    target_os = "macos",
    windows,
    all(unix, not(target_os = "macos"), not(target_os = "android"))
)))]
mod platform {
    use super::TextSettings;

    /// No known settings source on this OS.
    pub(super) fn read() -> TextSettings {
        TextSettings::default_for_os()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an XSETTINGS blob by hand in the given byte order.
    struct Blob {
        big: bool,
        bytes: Vec<u8>,
    }

    impl Blob {
        fn new(big: bool, serial: u32, count: u32) -> Self {
            let mut blob = Self {
                big,
                bytes: vec![u8::from(big), 0, 0, 0],
            };
            blob.u32(serial);
            blob.u32(count);
            blob
        }
        fn u16(&mut self, v: u16) {
            let b = if self.big { v.to_be_bytes() } else { v.to_le_bytes() };
            self.bytes.extend_from_slice(&b);
        }
        fn u32(&mut self, v: u32) {
            let b = if self.big { v.to_be_bytes() } else { v.to_le_bytes() };
            self.bytes.extend_from_slice(&b);
        }
        fn padded(&mut self, s: &str) {
            self.bytes.extend_from_slice(s.as_bytes());
            while !self.bytes.len().is_multiple_of(4) {
                self.bytes.push(0);
            }
        }
        fn head(&mut self, kind: u8, name: &str, serial: u32) {
            self.bytes.push(kind);
            self.bytes.push(0);
            self.u16(u16::try_from(name.len()).unwrap());
            self.padded(name);
            self.u32(serial);
        }
        fn int(mut self, name: &str, v: i32) -> Self {
            self.head(0, name, 7);
            self.u32(u32::from_ne_bytes(v.to_ne_bytes()));
            self
        }
        fn string(mut self, name: &str, v: &str) -> Self {
            self.head(1, name, 8);
            self.u32(u32::try_from(v.len()).unwrap());
            self.padded(v);
            self
        }
        fn color(mut self, name: &str, c: [u16; 4]) -> Self {
            self.head(2, name, 9);
            for channel in c {
                self.u16(channel);
            }
            self
        }
    }

    fn sample(big: bool) -> Vec<u8> {
        Blob::new(big, 42, 5)
            .int("Xft/Antialias", 1)
            .string("Xft/RGBA", "bgr")
            .int("Xft/DPI", -98_304)
            .color("Gtk/Color", [1, 2, 3, 0xffff])
            .string("Net/ThemeName", "Adwaita")
            .bytes
    }

    #[test]
    fn xsettings_both_byte_orders() {
        for big in [false, true] {
            let parsed = parse_xsettings(&sample(big)).unwrap();
            assert_eq!(parsed.serial, 42);
            assert_eq!(parsed.settings.len(), 5);
            assert_eq!(parsed.get("Xft/Antialias"), Some(&XValue::Int(1)));
            assert_eq!(parsed.get("Xft/RGBA"), Some(&XValue::Str("bgr".into())));
            assert_eq!(parsed.get("Xft/DPI"), Some(&XValue::Int(-98_304)));
            assert_eq!(parsed.get("Gtk/Color"), Some(&XValue::Color([1, 2, 3, 0xffff])));
            assert_eq!(parsed.get("Net/ThemeName"), Some(&XValue::Str("Adwaita".into())));
            assert_eq!(parsed.settings[1].serial, 8);
            assert_eq!(
                parsed.text_settings(),
                Some(plain(true, Some(Subpixel::Bgr))),
                "big endian: {big}"
            );
        }
    }

    #[test]
    fn xsettings_padding_exact_multiple_of_four() {
        // A four-byte name and string need no padding at all.
        let bytes = Blob::new(false, 1, 2).string("Xft", "vrgb").int("abcd", 3).bytes;
        let parsed = parse_xsettings(&bytes).unwrap();
        assert_eq!(parsed.get("Xft"), Some(&XValue::Str("vrgb".into())));
        assert_eq!(parsed.get("abcd"), Some(&XValue::Int(3)));
    }

    #[test]
    fn xsettings_malformed() {
        assert_eq!(parse_xsettings(&[]), None);
        assert_eq!(parse_xsettings(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        let full = sample(false);
        for cut in 1..full.len() {
            assert_eq!(parse_xsettings(&full[..cut]), None, "truncated at {cut}");
        }
        // A count larger than the entries present is malformed, not a hang.
        let mut lying = sample(true);
        lying[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(parse_xsettings(&lying), None);
    }

    #[test]
    fn xsettings_unknown_type_keeps_earlier_entries() {
        let mut blob = Blob::new(false, 1, 2).int("Xft/Antialias", 0);
        blob.head(9, "Weird", 1);
        let parsed = parse_xsettings(&blob.bytes).unwrap();
        assert_eq!(parsed.settings.len(), 1);
        assert_eq!(parsed.text_settings(), Some(plain(false, None)));
    }

    #[test]
    fn xsettings_without_font_keys() {
        let bytes = Blob::new(false, 1, 1).string("Net/ThemeName", "Adwaita").bytes;
        assert_eq!(parse_xsettings(&bytes).unwrap().text_settings(), None);
        assert_eq!(
            parse_xsettings(&Blob::new(true, 0, 0).bytes).unwrap(),
            XSettings::default()
        );
    }

    #[test]
    fn xft_mapping() {
        assert_eq!(from_xft(Some(1), Some("rgb")), plain(true, Some(Subpixel::Rgb)));
        assert_eq!(from_xft(Some(1), Some("vbgr")), plain(true, Some(Subpixel::VBgr)));
        assert_eq!(from_xft(Some(1), Some("none")), plain(true, None));
        assert_eq!(from_xft(Some(-1), Some("vrgb")), plain(true, Some(Subpixel::VRgb)));
        assert_eq!(from_xft(None, Some("bgr")), plain(true, Some(Subpixel::Bgr)));
        assert_eq!(from_xft(Some(0), Some("rgb")), plain(false, None));
        assert_eq!(from_xft(Some(1), None), plain(true, None));
        assert_eq!(from_xft(Some(1), Some("unknown")), plain(true, None));
    }

    #[test]
    fn gnome_mapping() {
        assert_eq!(from_gnome(Some("grayscale"), Some("bgr")), plain(true, None));
        assert_eq!(from_gnome(Some("rgba"), Some("bgr")), plain(true, Some(Subpixel::Bgr)));
        assert_eq!(
            from_gnome(Some("rgba"), Some("vrgb")),
            plain(true, Some(Subpixel::VRgb))
        );
        assert_eq!(
            from_gnome(Some("rgba"), Some("vbgr")),
            plain(true, Some(Subpixel::VBgr))
        );
        assert_eq!(from_gnome(Some("rgba"), None), plain(true, Some(Subpixel::Rgb)));
        assert_eq!(from_gnome(Some("none"), Some("rgb")), plain(false, None));
        assert_eq!(from_gnome(None, Some("rgb")), plain(true, None));
    }

    #[test]
    fn windows_mapping() {
        let defaults = from_windows(&WindowsRegistry::default());
        assert_eq!(
            defaults,
            TextSettings {
                stem_darkening: false,
                ..plain(true, Some(Subpixel::Rgb))
            }
        );

        let cleartype_bgr = WindowsRegistry {
            font_smoothing: Some(2),
            smoothing_type: Some(2),
            orientation: Some(0),
            smoothing_gamma: Some(1400),
            tuner: None,
        };
        // No tuner key: Chrome keeps Skia's gamma and contrast.
        assert_eq!(from_windows(&cleartype_bgr), plain(true, Some(Subpixel::Bgr)));

        let standard = WindowsRegistry {
            smoothing_type: Some(1),
            ..cleartype_bgr
        };
        assert_eq!(from_windows(&standard), plain(true, None));

        let off = WindowsRegistry {
            font_smoothing: Some(0),
            ..cleartype_bgr
        };
        assert_eq!(from_windows(&off), plain(false, None));

        let tuned = WindowsRegistry {
            tuner: Some(ClearTypeTuner {
                pixel_structure: Some(1),
                gamma_level: Some(1800),
                enhanced_contrast_level: Some(50),
            }),
            ..cleartype_bgr
        };
        let settings = from_windows(&tuned);
        assert_eq!(settings.subpixel, Some(Subpixel::Rgb), "tuner's PixelStructure wins");
        assert_eq!(settings.gamma, Some(1.8));
        assert_eq!(settings.contrast, Some(0.5));

        let flat = WindowsRegistry {
            tuner: Some(ClearTypeTuner {
                pixel_structure: Some(0),
                gamma_level: None,
                enhanced_contrast_level: Some(400),
            }),
            ..cleartype_bgr
        };
        let settings = from_windows(&flat);
        assert_eq!(settings.subpixel, None);
        assert_eq!(settings.gamma, Some(1.4), "falls back to FontSmoothingGamma");
        assert_eq!(settings.contrast, Some(1.0), "clamped to Skia's maximum");

        let wild = WindowsRegistry {
            smoothing_gamma: Some(5),
            tuner: Some(ClearTypeTuner {
                pixel_structure: Some(2),
                gamma_level: None,
                enhanced_contrast_level: None,
            }),
            ..cleartype_bgr
        };
        let settings = from_windows(&wild);
        assert_eq!(settings.subpixel, Some(Subpixel::Bgr));
        assert_eq!(settings.gamma, None, "out-of-range FontSmoothingGamma is ignored");
        assert_eq!(settings.contrast, None);

        let huge = WindowsRegistry {
            tuner: Some(ClearTypeTuner {
                pixel_structure: None,
                gamma_level: Some(u32::MAX),
                enhanced_contrast_level: None,
            }),
            ..cleartype_bgr
        };
        let settings = from_windows(&huge);
        assert!(settings.gamma.unwrap() < 4.0);
        assert_eq!(settings.subpixel, Some(Subpixel::Bgr), "no PixelStructure: orientation");
    }

    #[test]
    fn registry_numbers() {
        assert_eq!(parse_registry_number("2"), Some(2));
        assert_eq!(parse_registry_number(" 0 \0"), Some(0));
        assert_eq!(parse_registry_number("on"), None);
    }

    #[test]
    fn macos_mapping() {
        assert!(from_macos(None).stem_darkening);
        assert!(from_macos(Some(1)).stem_darkening);
        assert!(from_macos(Some(2)).stem_darkening);
        assert!(!from_macos(Some(0)).stem_darkening);
        assert_eq!(from_macos(None).subpixel, None);
        assert!(from_macos(Some(0)).antialias);
    }

    #[test]
    fn override_values() {
        let base = TextSettings::default_for_os();
        assert_eq!(parse_override("gray", base).unwrap().subpixel, None);
        assert_eq!(parse_override("lcd-rgb", base).unwrap().subpixel, Some(Subpixel::Rgb));
        assert_eq!(parse_override("LCD-BGR", base).unwrap().subpixel, Some(Subpixel::Bgr));
        assert_eq!(parse_override("lcd-vrgb", base).unwrap().subpixel, Some(Subpixel::VRgb));
        assert_eq!(parse_override("lcd-vbgr", base).unwrap().subpixel, Some(Subpixel::VBgr));
        let none = parse_override("none", base).unwrap();
        assert!(!none.antialias);
        assert_eq!(none.subpixel, None);
        assert_eq!(parse_override("subpixel", base), None);
        assert_eq!(
            parse_override("gray", base).unwrap().stem_darkening,
            base.stem_darkening
        );
    }

    #[test]
    fn rgba_names() {
        assert_eq!(parse_rgba("rgb"), Some(Subpixel::Rgb));
        assert_eq!(parse_rgba("none"), None);
        assert_eq!(parse_rgba("RGB"), None, "fontconfig names are lowercase");
    }

    #[test]
    fn deadline_gives_up() {
        let start = std::time::Instant::now();
        let slow = with_deadline(Duration::from_millis(20), || {
            std::thread::sleep(Duration::from_millis(500));
            Some(1)
        });
        assert_eq!(slow, None);
        assert!(start.elapsed() < Duration::from_millis(400));
        assert_eq!(with_deadline(Duration::from_secs(5), || Some(2)), Some(2));
    }

    #[test]
    fn read_this_machine() {
        let start = std::time::Instant::now();
        let settings = read();
        println!("read() = {settings:?} in {:?}", start.elapsed());
        println!("read_with_override() = {:?}", read_with_override());
    }
}
