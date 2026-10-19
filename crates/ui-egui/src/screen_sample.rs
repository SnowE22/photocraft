//! The Eyedropper outside the image (a user request, as in Photoshop): press on the canvas and
//! drag anywhere, over PhotoCraft's own panels or another application, and the colour under the
//! pointer is read from the screen.
//!
//! Windows only for now: the pointer's position from `GetCursorPos` (winsafe) and a Sample Size
//! square of the desktop from GDI (xcap's `capture_region`), both safe APIs, in physical pixels
//! (the app is per-monitor DPI aware). Elsewhere [`under_cursor`] answers `None` and the
//! Eyedropper samples the image only.

/// The average of the `size × size` square of the screen centred on the pointer, as sRGB 0–1,
/// cut to the monitor the pointer is on. `None` when it can't be read.
#[cfg(target_os = "windows")]
pub fn under_cursor(size: u32) -> Option<[f32; 3]> {
    let p = winsafe::GetCursorPos().ok()?;
    let m = xcap::Monitor::from_point(p.x, p.y).ok()?;
    let (mx, my, mw, mh) = (m.x().ok()?, m.y().ok()?, m.width().ok()?, m.height().ok()?);
    let (x, y, w, h) = square([p.x - mx, p.y - my], size, [mw, mh])?;
    let img = m.capture_region(x, y, w, h).ok()?;
    average(img.as_raw())
}

#[cfg(not(target_os = "windows"))]
pub fn under_cursor(_size: u32) -> Option<[f32; 3]> {
    None
}

/// The square of `size` pixels (1–101) centred on monitor pixel `at`, cut to a monitor of
/// `monitor` pixels: (x, y, width, height). `None` when `at` is off the monitor.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn square(at: [i32; 2], size: u32, monitor: [u32; 2]) -> Option<(u32, u32, u32, u32)> {
    let (mw, mh) = (i64::from(monitor[0]), i64::from(monitor[1]));
    let (x, y) = (i64::from(at[0]), i64::from(at[1]));
    if x < 0 || y < 0 || x >= mw || y >= mh {
        return None;
    }
    let half = i64::from(size.clamp(1, 101) / 2);
    let (x0, y0) = ((x - half).max(0), (y - half).max(0));
    let (x1, y1) = ((x + half + 1).min(mw), (y + half + 1).min(mh));
    Some((u32::try_from(x0).ok()?, u32::try_from(y0).ok()?, u32::try_from(x1 - x0).ok()?, u32::try_from(y1 - y0).ok()?))
}

/// The mean colour of RGBA8 pixels, 0–1. `None` for no pixels.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn average(rgba: &[u8]) -> Option<[f32; 3]> {
    let (px, _) = rgba.as_chunks::<4>();
    if px.is_empty() {
        return None;
    }
    let mut sum = [0u64; 3];
    for p in px {
        for (s, v) in sum.iter_mut().zip(p) {
            *s += u64::from(*v);
        }
    }
    let n = px.len() as f64 * 255.0;
    Some(sum.map(|s| (s as f64 / n) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_square_is_centred_and_cut_to_the_monitor() {
        assert_eq!(square([100, 50], 1, [1920, 1080]), Some((100, 50, 1, 1)));
        assert_eq!(square([100, 50], 5, [1920, 1080]), Some((98, 48, 5, 5)));
        assert_eq!(square([0, 0], 5, [1920, 1080]), Some((0, 0, 3, 3)), "cut at the corner");
        assert_eq!(square([1919, 1079], 11, [1920, 1080]), Some((1914, 1074, 6, 6)));
        assert_eq!(square([-1, 5], 3, [1920, 1080]), None, "off the monitor");
        assert_eq!(square([5, 5], 0, [0, 0]), None);
        assert_eq!(square([i32::MAX, i32::MIN], u32::MAX, [u32::MAX, u32::MAX]), None);
    }

    #[test]
    fn the_average_ignores_alpha() {
        assert_eq!(average(&[255, 0, 0, 255, 0, 0, 255, 0]), Some([0.5, 0.0, 0.5]));
        assert_eq!(average(&[]), None);
        assert_eq!(average(&[1, 2, 3]), None, "a partial pixel");
    }
}
