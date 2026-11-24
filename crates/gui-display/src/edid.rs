//! What a monitor says of itself in its EDID base block: who made it, its
//! name, its primaries and the vertical rates it takes. X11 hands the block
//! over as an output property; nothing else on X11 tells the last two.

use std::time::Duration;

use crate::gamut::Primaries;

/// The parts of an EDID base block this crate reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Edid {
    /// Manufacturer, product code and serial number, as the block holds
    /// them (bytes 8 to 15): the same for one monitor wherever it is
    /// plugged in. Two monitors of one model without serial numbers share
    /// it.
    pub identity: [u8; 8],
    /// The monitor's name descriptor, when it has one.
    pub name: Option<String>,
    /// The panel's own chromaticities: red, green, blue.
    pub primaries: Primaries,
    /// The lowest and highest vertical rates it takes, in Hz, from its
    /// range limits descriptor, when it has one.
    pub vertical: Option<(u16, u16)>,
}

const MAGIC: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// Read a base block; none when the bytes are not one.
#[must_use]
pub fn parse(bytes: &[u8]) -> Option<Edid> {
    let block = bytes.get(..128)?;
    if block.get(..8)? != MAGIC {
        return None;
    }
    let identity: [u8; 8] = block.get(8..16)?.try_into().ok()?;
    // Ten bits a coordinate: the high eight in a byte of their own, the low
    // two packed in bytes 25 and 26.
    let low = (*block.get(25)?, *block.get(26)?);
    let coordinate = |high: usize, low: u8, shift: u32| -> Option<f64> {
        let high = u16::from(*block.get(high)?);
        let low = u16::from(low.checked_shr(shift)? & 0b11);
        Some(f64::from(high.checked_shl(2)? | low) / 1024.0)
    };
    let primaries = [
        (coordinate(27, low.0, 6)?, coordinate(28, low.0, 4)?),
        (coordinate(29, low.0, 2)?, coordinate(30, low.0, 0)?),
        (coordinate(31, low.1, 6)?, coordinate(32, low.1, 4)?),
    ];
    let mut name = None;
    let mut vertical = None;
    for at in [54usize, 72, 90, 108] {
        let d = block.get(at..at.checked_add(18)?)?;
        // A display descriptor starts with three zero bytes; a timing does not.
        if d.get(..3)? != [0, 0, 0] {
            continue;
        }
        match *d.get(3)? {
            0xfc => {
                let text: Vec<u8> = d.get(5..)?.iter().copied().take_while(|b| *b != 0x0a).collect();
                let text = String::from_utf8_lossy(&text).trim().to_owned();
                name = (!text.is_empty()).then_some(text);
            }
            0xfd => {
                // EDID 1.4: bits 0 and 1 of byte 4 add 255 to the lowest
                // and highest rates.
                let offsets = *d.get(4)?;
                let rate = |byte: usize, bit: u8| -> Option<u16> {
                    let base = u16::from(*d.get(byte)?);
                    Some(if offsets & bit == 0 {
                        base
                    } else {
                        base.checked_add(255)?
                    })
                };
                let (lowest, highest) = (rate(5, 0b01)?, rate(6, 0b10)?);
                vertical = (lowest > 0 && highest >= lowest).then_some((lowest, highest));
            }
            _ => {}
        }
    }
    Some(Edid {
        identity,
        name,
        primaries,
        vertical,
    })
}

/// A mode's refresh interval from its timing, as RandR gives it: the pixel
/// clock in Hz over the pixels of a whole frame, blanking included. An
/// interlaced mode draws two fields a frame, a double-scanned one each line
/// twice. None for a mode with no timing (a virtual output).
#[must_use]
pub fn refresh_interval(
    dot_clock: u32,
    htotal: u16,
    vtotal: u16,
    interlaced: bool,
    double_scan: bool,
) -> Option<Duration> {
    if dot_clock == 0 || htotal == 0 || vtotal == 0 {
        return None;
    }
    let mut frame = f64::from(htotal) * f64::from(vtotal);
    if interlaced {
        frame /= 2.0;
    }
    if double_scan {
        frame *= 2.0;
    }
    let seconds = frame / f64::from(dot_clock);
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A base block of a monitor named `name`, with sRGB primaries and the
    /// vertical range given.
    #[expect(clippy::arithmetic_side_effects, reason = "a test's offsets into a fixed block")]
    pub(crate) fn block(name: &str, vertical: Option<(u8, u8)>) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[..8].copy_from_slice(&MAGIC);
        b[8..16].copy_from_slice(&[0x10, 0xac, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12]);
        // sRGB: red 0.640, 0.330; green 0.300, 0.600; blue 0.150, 0.060.
        let ten = |v: f64| -> u16 {
            #[expect(clippy::as_conversions, reason = "a test's ten-bit number")]
            let n = (v * 1024.0).round() as u16;
            n
        };
        let c = [0.640, 0.330, 0.300, 0.600, 0.150, 0.060].map(ten);
        b[25] = u8::try_from(((c[0] & 3) << 6) | ((c[1] & 3) << 4) | ((c[2] & 3) << 2) | (c[3] & 3)).unwrap();
        b[26] = u8::try_from(((c[4] & 3) << 6) | ((c[5] & 3) << 4)).unwrap();
        for (i, v) in c.iter().enumerate() {
            b[27 + i] = u8::try_from(v >> 2).unwrap();
        }
        // A timing descriptor first, then the name, then the range.
        b[54] = 0x01;
        b[72 + 3] = 0xfc;
        let mut text = name.as_bytes().to_vec();
        text.push(0x0a);
        text.resize(13, b' ');
        b[72 + 5..72 + 18].copy_from_slice(&text);
        if let Some((lowest, highest)) = vertical {
            b[90 + 3] = 0xfd;
            b[90 + 5] = lowest;
            b[90 + 6] = highest;
        } else {
            b[90] = 0x01;
        }
        b[108] = 0x01;
        b
    }

    #[test]
    fn a_block_gives_identity_name_primaries_and_range() {
        let e = parse(&block("DELL U2720Q", Some((48, 144)))).unwrap();
        assert_eq!(e.identity, [0x10, 0xac, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(e.name.as_deref(), Some("DELL U2720Q"));
        assert_eq!(e.vertical, Some((48, 144)));
        for (have, want) in e.primaries.iter().zip([(0.640, 0.330), (0.300, 0.600), (0.150, 0.060)]) {
            assert!(
                (have.0 - want.0).abs() < 0.001 && (have.1 - want.1).abs() < 0.001,
                "{have:?} {want:?}"
            );
        }
        assert_eq!(crate::gamut::of_native(&e.primaries), crate::Gamut::Srgb);
    }

    #[test]
    fn a_rate_offset_adds_255() {
        let mut b = block("FAST", Some((48, 105)));
        // The highest rate is 105 + 255 = 360 Hz.
        b[90 + 4] = 0b10;
        assert_eq!(parse(&b).unwrap().vertical, Some((48, 360)));
    }

    #[test]
    fn a_block_without_descriptors_has_no_name_or_range() {
        let mut b = block("X", None);
        b[72 + 3] = 0x10;
        let e = parse(&b).unwrap();
        assert_eq!((e.name, e.vertical), (None, None));
    }

    #[test]
    fn bytes_that_are_no_block_are_none() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[0; 128]), None);
        assert_eq!(parse(&block("X", None)[..100]), None);
    }

    #[test]
    fn a_mode_refreshes_at_its_clock_over_its_frame() {
        // 1920×1080 at 60 Hz (CEA): 148.5 MHz over 2200 × 1125.
        let i = refresh_interval(148_500_000, 2200, 1125, false, false).unwrap();
        assert!((i.as_secs_f64() - 1.0 / 60.0).abs() < 1e-9);
        // The same timing interlaced shows 120 fields a second.
        let i = refresh_interval(148_500_000, 2200, 1125, true, false).unwrap();
        assert!((i.as_secs_f64() - 1.0 / 120.0).abs() < 1e-9);
        assert_eq!(refresh_interval(0, 2200, 1125, false, false), None);
        assert_eq!(refresh_interval(148_500_000, 0, 1125, false, false), None);
    }
}
