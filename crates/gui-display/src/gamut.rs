//! Which standard gamut a display's primaries are, from what the OS holds
//! about it: its ICC profile (macOS) or its EDID (X11).
//!
//! The two describe primaries differently. An ICC profile holds colorants
//! adapted to D50; an EDID holds the panel's own chromaticities. Each is
//! compared with the standard gamuts in its own terms, so nothing here
//! adapts between white points.

/// The standard gamut an output's primaries match, if one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gamut {
    Srgb,
    DisplayP3,
    AdobeRgb,
    Rec2020,
    /// The OS told nothing, or primaries near no standard gamut.
    Unknown,
}

/// Red, green and blue as CIE xy chromaticities.
pub type Primaries = [(f64, f64); 3];

/// How far a coordinate may be from a standard's and still be it: panels
/// are measured, standards are not.
const NEAR: f64 = 0.03;

/// The standard gamuts' own chromaticities (what an EDID is compared with).
const NATIVE: [(Gamut, Primaries); 4] = [
    (Gamut::Srgb, [(0.640, 0.330), (0.300, 0.600), (0.150, 0.060)]),
    (Gamut::DisplayP3, [(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)]),
    (Gamut::AdobeRgb, [(0.640, 0.330), (0.210, 0.710), (0.150, 0.060)]),
    (Gamut::Rec2020, [(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)]),
];

/// The same gamuts' colorants adapted to D50 (Bradford), as an ICC profile
/// holds them.
const ADAPTED: [(Gamut, Primaries); 4] = [
    (Gamut::Srgb, [(0.6485, 0.3309), (0.3212, 0.5979), (0.1559, 0.0660)]),
    (Gamut::DisplayP3, [(0.6821, 0.3194), (0.2846, 0.6746), (0.1559, 0.0661)]),
    (Gamut::AdobeRgb, [(0.6484, 0.3309), (0.2302, 0.7015), (0.1559, 0.0660)]),
    (Gamut::Rec2020, [(0.7085, 0.2935), (0.1901, 0.7754), (0.1293, 0.0471)]),
];

/// The gamut whose primaries are nearest `p`, each coordinate within
/// [`NEAR`].
fn nearest(p: &Primaries, table: &[(Gamut, Primaries)]) -> Gamut {
    let mut best = (Gamut::Unknown, f64::INFINITY);
    for (gamut, standard) in table {
        let mut distance = 0.0;
        let mut near = true;
        for (have, want) in p.iter().zip(standard) {
            let (dx, dy) = (have.0 - want.0, have.1 - want.1);
            near &= dx.abs() <= NEAR && dy.abs() <= NEAR;
            distance += dx * dx + dy * dy;
        }
        if near && distance < best.1 {
            best = (*gamut, distance);
        }
    }
    best.0
}

/// The gamut of a panel with these chromaticities of its own (an EDID's).
#[must_use]
pub fn of_native(p: &Primaries) -> Gamut {
    nearest(p, &NATIVE)
}

/// The gamut of an ICC display profile, from its `rXYZ`, `gXYZ` and `bXYZ`
/// colorants; [`Gamut::Unknown`] for a profile without them (a table-based
/// one) or bytes that are no profile.
#[must_use]
pub fn of_icc(profile: &[u8]) -> Gamut {
    let colorant = |tag: &[u8; 4]| -> Option<(f64, f64)> {
        let (x, y, z) = icc_xyz(profile, tag)?;
        let sum = x + y + z;
        (sum > 0.0).then(|| (x / sum, y / sum))
    };
    match (colorant(b"rXYZ"), colorant(b"gXYZ"), colorant(b"bXYZ")) {
        (Some(r), Some(g), Some(b)) => nearest(&[r, g, b], &ADAPTED),
        _ => Gamut::Unknown,
    }
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(b.try_into().ok()?))
}

/// The XYZ value of the profile's tag `tag` (an `XYZType`: a signature,
/// four reserved bytes, three s15Fixed16 numbers).
fn icc_xyz(profile: &[u8], tag: &[u8; 4]) -> Option<(f64, f64, f64)> {
    const HEADER: usize = 128;
    if profile.get(36..40)? != b"acsp" {
        return None;
    }
    let count = usize::try_from(be_u32(profile, HEADER)?).ok()?;
    let table = HEADER.checked_add(4)?;
    // A profile holds a few dozen tags; a count beyond the bytes is no profile.
    for i in 0..count.min(profile.len() / 12) {
        let entry = table.checked_add(i.checked_mul(12)?)?;
        if profile.get(entry..entry.checked_add(4)?)? != tag {
            continue;
        }
        let at = usize::try_from(be_u32(profile, entry.checked_add(4)?)?).ok()?;
        if profile.get(at..at.checked_add(4)?)? != b"XYZ " {
            return None;
        }
        let fixed = |n: usize| -> Option<f64> {
            let raw = be_u32(profile, at.checked_add(8)?.checked_add(n.checked_mul(4)?)?)?;
            Some(f64::from(i32::from_be_bytes(raw.to_be_bytes())) / 65536.0)
        };
        return Some((fixed(0)?, fixed(1)?, fixed(2)?));
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal ICC profile holding the three colorants given as XYZ.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "a test's offsets into a profile it builds"
    )]
    pub(crate) fn profile(colorants: [(f64, f64, f64); 3]) -> Vec<u8> {
        let mut p = vec![0u8; 128];
        p[36..40].copy_from_slice(b"acsp");
        p.extend_from_slice(&3u32.to_be_bytes());
        let data = 128 + 4 + 3 * 12;
        for (i, tag) in [b"rXYZ", b"gXYZ", b"bXYZ"].iter().enumerate() {
            p.extend_from_slice(*tag);
            p.extend_from_slice(&u32::try_from(data + i * 20).unwrap().to_be_bytes());
            p.extend_from_slice(&20u32.to_be_bytes());
        }
        for (x, y, z) in colorants {
            p.extend_from_slice(b"XYZ ");
            p.extend_from_slice(&[0; 4]);
            for v in [x, y, z] {
                #[expect(clippy::as_conversions, reason = "a test's fixed-point number")]
                let fixed = (v * 65536.0).round() as i32;
                p.extend_from_slice(&fixed.to_be_bytes());
            }
        }
        p
    }

    #[test]
    fn icc_colorants_name_their_gamut() {
        // The colorants of the sRGB v4 and Display P3 profiles.
        let srgb = profile([
            (0.4361, 0.2225, 0.0139),
            (0.3851, 0.7169, 0.0971),
            (0.1431, 0.0606, 0.7141),
        ]);
        let p3 = profile([
            (0.5151, 0.2412, -0.0011),
            (0.2920, 0.6922, 0.0419),
            (0.1571, 0.0666, 0.7841),
        ]);
        let rec2020 = profile([
            (0.6734, 0.2790, -0.0019),
            (0.1656, 0.6753, 0.0300),
            (0.1251, 0.0456, 0.7969),
        ]);
        assert_eq!(of_icc(&srgb), Gamut::Srgb);
        assert_eq!(of_icc(&p3), Gamut::DisplayP3);
        assert_eq!(of_icc(&rec2020), Gamut::Rec2020);
    }

    #[test]
    fn a_panel_near_a_standard_is_that_standard_and_one_far_from_all_is_unknown() {
        // A calibrated laptop panel: P3 within a few thousandths.
        let panel = profile([
            (0.5120, 0.2420, 0.0000),
            (0.2950, 0.6900, 0.0430),
            (0.1570, 0.0680, 0.7800),
        ]);
        assert_eq!(of_icc(&panel), Gamut::DisplayP3);
        let odd = profile([(0.30, 0.30, 0.30), (0.2920, 0.6922, 0.0419), (0.1571, 0.0666, 0.7841)]);
        assert_eq!(of_icc(&odd), Gamut::Unknown);
    }

    #[test]
    fn bytes_that_are_no_profile_are_unknown_and_never_panic() {
        assert_eq!(of_icc(&[]), Gamut::Unknown);
        assert_eq!(of_icc(&[0xff; 200]), Gamut::Unknown);
        let mut cut = profile([
            (0.4361, 0.2225, 0.0139),
            (0.3851, 0.7169, 0.0971),
            (0.1431, 0.0606, 0.7141),
        ]);
        cut.truncate(190);
        assert_eq!(of_icc(&cut), Gamut::Unknown);
        // A tag count far beyond the bytes, and an offset beyond them.
        let mut lying = profile([
            (0.4361, 0.2225, 0.0139),
            (0.3851, 0.7169, 0.0971),
            (0.1431, 0.0606, 0.7141),
        ]);
        lying[128..132].copy_from_slice(&u32::MAX.to_be_bytes());
        lying[136..140].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(of_icc(&lying), Gamut::Unknown);
    }

    #[test]
    fn native_primaries_name_their_gamut() {
        assert_eq!(of_native(&[(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)]), Gamut::Srgb);
        assert_eq!(of_native(&[(0.652, 0.335), (0.305, 0.62), (0.148, 0.055)]), Gamut::Srgb);
        assert_eq!(
            of_native(&[(0.68, 0.32), (0.265, 0.69), (0.15, 0.06)]),
            Gamut::DisplayP3
        );
        assert_eq!(
            of_native(&[(0.708, 0.292), (0.17, 0.797), (0.131, 0.046)]),
            Gamut::Rec2020
        );
        assert_eq!(of_native(&[(0.5, 0.5), (0.3, 0.6), (0.15, 0.06)]), Gamut::Unknown);
    }
}
