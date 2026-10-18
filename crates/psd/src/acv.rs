//! Photoshop Curves files (`.acv`), the presets the Curves dialog saves and loads.
//!
//! Layout from Adobe's public "Photoshop File Formats Specification" (Curves file format): a
//! `u16` version (1, or 4 since Photoshop CS), a `u16` count of curves, then each curve as a
//! `u16` point count (the dialog allows 2 to 19) and that many points, each two `u16`s: the
//! output value, then the input value, both 0..=255. An RGB document's file holds the composite
//! curve, then red, green and blue (and a fifth curve Photoshop writes for CMYK's black); a
//! grayscale document's holds one. Observed in the files Photoshop 2022–2026 saves: version 4
//! with five curves and nothing after them.
//!
//! [`parse`] never panics: counts are checked against the bytes left and capped.

use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt};

/// Most curves read from one file (the dialog's channels: at most five).
pub const MAX_CURVES: usize = 64;
/// Most points read from one curve (the dialog's own limit is 19).
pub const MAX_POINTS: usize = 256;

/// One curve: its points as `(input, output)`, both 0..=255, in file order.
pub type Curve = Vec<(u16, u16)>;

/// Reads a `.acv` file: its curves, composite first.
pub fn parse(data: &[u8]) -> Result<Vec<Curve>> {
    let mut r = Reader::new(data);
    let version = r.u16()?;
    if version != 1 && version != 4 {
        return Err(PsdError::Unsupported(format!("curves file version {version} (1 and 4 are supported)")));
    }
    let count = usize::from(r.u16()?);
    if count > MAX_CURVES {
        return Err(PsdError::LimitExceeded("too many curves in one file"));
    }
    // Each curve needs at least its point count.
    r.check_count(count as u64, 2)?;
    let mut curves = Vec::with_capacity(count);
    for _ in 0..count {
        let n = usize::from(r.u16()?);
        if n > MAX_POINTS {
            return Err(PsdError::LimitExceeded("too many points in one curve"));
        }
        r.check_count(n as u64, 4)?;
        let mut curve = Vec::with_capacity(n);
        for _ in 0..n {
            let output = r.u16()?;
            let input = r.u16()?;
            if output > 255 || input > 255 {
                return Err(PsdError::invalid(format!("curve point ({input}, {output}) is outside 0..255")));
            }
            curve.push((input, output));
        }
        curves.push(curve);
    }
    Ok(curves)
}

/// Writes curves (composite first) as a version 4 `.acv` file.
pub fn write(curves: &[Curve]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.put_u16(4);
    out.put_u16(u16::try_from(curves.len()).map_err(|_| PsdError::LimitExceeded("too many curves"))?);
    for c in curves {
        out.put_u16(u16::try_from(c.len()).map_err(|_| PsdError::LimitExceeded("too many points"))?);
        for &(input, output) in c {
            out.put_u16(output);
            out.put_u16(input);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Photoshop 2026 `.acv`: version 4, five curves, the composite lifting 50 to 66.
    const GREY_FIX: [u8; 58] = [
        0, 4, 0, 5, //
        0, 3, 0, 0, 0, 0, 0, 0x42, 0, 0x32, 0, 0xff, 0, 0xff, //
        0, 2, 0, 0, 0, 0, 0, 0xff, 0, 0xff, //
        0, 2, 0, 0, 0, 0, 0, 0xff, 0, 0xff, //
        0, 2, 0, 0, 0, 0, 0, 0xff, 0, 0xff, //
        0, 2, 0, 0, 0, 0, 0, 0xff, 0, 0xff,
    ];

    #[test]
    fn reads_what_photoshop_saves() {
        let curves = parse(&GREY_FIX).unwrap();
        assert_eq!(curves.len(), 5);
        assert_eq!(curves[0], [(0, 0), (50, 66), (255, 255)], "input first, output second");
        assert!(curves[1..].iter().all(|c| *c == [(0, 0), (255, 255)]));
        assert_eq!(write(&curves).unwrap(), GREY_FIX, "writes the same bytes back");
    }

    #[test]
    fn version_one_and_truncation() {
        let mut v1 = GREY_FIX;
        v1[1] = 1;
        assert_eq!(parse(&v1).unwrap().len(), 5);
        for cut in 0..GREY_FIX.len() {
            assert!(parse(&GREY_FIX[..cut]).is_err(), "cut at {cut}");
        }
        let mut v9 = GREY_FIX;
        v9[1] = 9;
        assert!(matches!(parse(&v9), Err(PsdError::Unsupported(_))));
        // Counts far beyond the data fail before allocating; out-of-range points are refused.
        assert!(parse(&[0, 4, 0xff, 0xff]).is_err());
        assert!(parse(&[0, 4, 0, 1, 0xff, 0xff]).is_err());
        assert!(parse(&[0, 4, 0, 1, 0, 1, 1, 0, 0, 0]).is_err(), "output 256");
    }

    #[test]
    fn garbage_never_panics() {
        let mut x: u32 = 0x2545_f491;
        for i in 0..2000 {
            let mut b = GREY_FIX.to_vec();
            for _ in 0..(i % 4 + 1) {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let at = (x as usize) % b.len();
                b[at] = (x >> 8) as u8;
            }
            let _ = parse(&b);
        }
    }
}
