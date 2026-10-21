use super::*;

const CLEAR: [u8; 4] = [0, 0, 0, 0];
const INK: [u8; 4] = [0, 0, 0, 255];

struct Img {
    w: usize,
    px: Vec<[u8; 4]>,
}

impl Img {
    fn new(w: usize, h: usize, c: [u8; 4]) -> Self {
        Img { w, px: vec![c; w * h] }
    }
    fn area(&self) -> Rect {
        Rect::new(0, 0, self.w as i32, (self.px.len() / self.w) as i32)
    }
    fn set(&mut self, x: usize, y: usize, c: [u8; 4]) {
        self.px[y * self.w + x] = c;
    }
    /// A one-pixel outline of the box `x0..=x1` × `y0..=y1`.
    fn outline(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        for x in x0..=x1 {
            self.set(x, y0, INK);
            self.set(x, y1, INK);
        }
        for y in y0..=y1 {
            self.set(x0, y, INK);
            self.set(x1, y, INK);
        }
    }
    fn click(&self, x: i32, y: i32, o: FillOptions) -> Option<Region> {
        click_region(&self.px, self.area(), (x, y), &o)
    }
}

fn hard() -> FillOptions {
    FillOptions { anti_alias: false, ..FillOptions::default() }
}

fn filled(r: &Region, x: i32, y: i32) -> bool {
    r.at(x, y) > 0.5
}

/// A box with a 3 px gap in its top edge.
fn open_box(gap: usize) -> Img {
    let mut img = Img::new(40, 40, CLEAR);
    img.outline(5, 5, 34, 34);
    for x in 18..18 + gap {
        img.set(x, 5, CLEAR);
    }
    img
}

#[test]
fn close_gap_stops_a_fill_at_a_gap_in_the_lines() {
    let img = open_box(3);
    let leaked = img.click(20, 20, hard()).unwrap();
    assert!(filled(&leaked, 0, 0), "without Close Gap the fill runs out of the box");
    let closed = img.click(20, 20, FillOptions { close_gap: 4.0, ..hard() }).unwrap();
    assert!(!filled(&closed, 0, 0) && !filled(&closed, 19, 2), "the gap is closed");
    // The whole inside is filled, right up to the lines and into the corners.
    for (x, y) in [(20, 20), (6, 6), (33, 33), (6, 33), (19, 6), (17, 6), (21, 6)] {
        assert!(filled(&closed, x, y), "({x}, {y})");
    }
    // Clicking outside fills the outside and stops at the same gap.
    let outside = img.click(1, 1, FillOptions { close_gap: 4.0, ..hard() }).unwrap();
    assert!(filled(&outside, 19, 4) && !filled(&outside, 20, 20) && !filled(&outside, 19, 6));
    // Together the two fills leave no speck: the seam closing the gap is covered by both.
    for y in 0..40 {
        for x in 0..40 {
            if img.px[y * 40 + x] == CLEAR {
                assert!(filled(&closed, x as i32, y as i32) || filled(&outside, x as i32, y as i32), "({x}, {y}) left unfilled");
            }
        }
    }
}

#[test]
fn a_gap_wider_than_close_gap_still_leaks() {
    let img = open_box(9);
    let r = img.click(20, 20, FillOptions { close_gap: 4.0, ..hard() }).unwrap();
    assert!(filled(&r, 0, 0));
}

#[test]
fn a_click_in_a_spot_narrower_than_the_gap_fills_that_spot() {
    let mut img = Img::new(40, 20, CLEAR);
    img.outline(5, 5, 30, 9); // inside: 3 px tall
    let r = img.click(15, 7, FillOptions { close_gap: 10.0, ..hard() }).unwrap();
    for (x, y) in [(15, 7), (6, 6), (29, 8)] {
        assert!(filled(&r, x, y), "({x}, {y})");
    }
    assert!(!filled(&r, 0, 0) && !filled(&r, 5, 7));
}

#[test]
fn the_canvas_edge_closes_gaps_when_referred_to() {
    // A wall down the canvas that stops 2 px short of the top edge.
    let mut img = Img::new(30, 20, CLEAR);
    for y in 2..20 {
        img.set(15, y, INK);
    }
    let o = FillOptions { close_gap: 4.0, ..hard() };
    assert!(filled(&img.click(3, 10, o).unwrap(), 25, 10), "the edge isn't a line by default");
    let r = img.click(3, 10, FillOptions { image_border: true, ..o }).unwrap();
    assert!(!filled(&r, 25, 10) && filled(&r, 3, 1));
}

/// An anti-aliased line down x = 10..=14: alpha 64, 160, 255, 160, 64.
fn soft_line() -> Img {
    let mut img = Img::new(30, 10, CLEAR);
    for y in 0..10 {
        for (x, a) in [(10, 64), (11, 160), (12, 255), (13, 160), (14, 64)] {
            img.set(x, y, [0, 0, 0, a]);
        }
    }
    img
}

#[test]
fn to_darkest_pixel_tucks_the_fill_under_the_line_and_no_further() {
    let img = soft_line();
    let plain = img.click(2, 5, hard()).unwrap();
    assert!(filled(&plain, 9, 5) && !filled(&plain, 10, 5));
    let r = img.click(2, 5, FillOptions { scale: 5.0, ..hard() }).unwrap();
    for x in [9, 10, 11, 12] {
        assert!(filled(&r, x, 5), "x = {x}");
    }
    for x in [13, 14, 15, 20] {
        assert!(!filled(&r, x, 5), "x = {x}: past the line's core");
    }
    // Only as far as asked.
    let short = img.click(2, 5, FillOptions { scale: 1.0, ..hard() }).unwrap();
    assert!(filled(&short, 10, 5) && !filled(&short, 11, 5));
}

#[test]
fn round_and_square_scaling_grow_by_distance() {
    let img = soft_line();
    let r = img.click(2, 5, FillOptions { scale: 5.0, scaling: Scaling::Round, ..hard() }).unwrap();
    assert!(filled(&r, 14, 5) && !filled(&r, 15, 5), "5 px past the edge at x = 9, whatever the pixels");
    // One open pixel in solid ink: the shapes the growth makes.
    let mut img = Img::new(21, 21, INK);
    img.set(10, 10, CLEAR);
    let square = img.click(10, 10, FillOptions { scale: 2.0, scaling: Scaling::Square, ..hard() }).unwrap();
    assert!(filled(&square, 12, 12) && filled(&square, 8, 8) && !filled(&square, 13, 10));
    let round = img.click(10, 10, FillOptions { scale: 2.0, scaling: Scaling::Round, ..hard() }).unwrap();
    assert!(filled(&round, 12, 10) && filled(&round, 11, 11) && !filled(&round, 12, 12));
}

#[test]
fn negative_scaling_shrinks_the_fill() {
    let mut img = Img::new(20, 20, CLEAR);
    img.outline(5, 5, 15, 15);
    let r = img.click(10, 10, FillOptions { scale: -2.0, scaling: Scaling::Round, ..hard() }).unwrap();
    assert!(!filled(&r, 6, 10) && !filled(&r, 7, 10) && filled(&r, 8, 10) && filled(&r, 10, 10));
}

#[test]
fn without_follow_adjacent_every_similar_pixel_is_filled() {
    let mut img = Img::new(40, 20, CLEAR);
    img.outline(2, 2, 10, 10);
    img.outline(20, 2, 30, 10);
    let one = img.click(5, 5, hard()).unwrap();
    assert!(filled(&one, 5, 5) && !filled(&one, 25, 5));
    let all = img.click(5, 5, FillOptions { contiguous: false, ..hard() }).unwrap();
    assert!(filled(&all, 25, 5) && filled(&all, 0, 0) && !filled(&all, 2, 5));
}

#[test]
fn color_margin_decides_what_the_fill_crosses() {
    let mut img = Img::new(20, 5, [255, 255, 255, 255]);
    for y in 0..5 {
        img.set(10, y, [200, 200, 200, 255]); // a faint grey line
    }
    assert!(!filled(&img.click(2, 2, hard()).unwrap(), 15, 2), "10 % stops at it");
    let r = img.click(2, 2, FillOptions { tolerance: 25.0, ..hard() }).unwrap();
    assert!(filled(&r, 15, 2), "25 % crosses it");
}

#[test]
fn enclose_and_fill_fills_only_areas_wholly_inside() {
    let mut img = Img::new(40, 40, CLEAR);
    img.outline(5, 5, 15, 15);
    img.outline(20, 5, 30, 15);
    // A lasso round the first box and half of the second.
    let lasso = lasso_mask(&[(2.0, 2.0), (25.0, 2.0), (25.0, 18.0), (2.0, 18.0)], img.area());
    let o = hard();
    let r = enclosed_region(&img.px, img.area(), &lasso, Target::Transparent, &o).unwrap();
    assert!(filled(&r, 10, 10) && filled(&r, 6, 6));
    assert!(!filled(&r, 22, 10), "the second box isn't wholly inside");
    assert!(!filled(&r, 3, 3), "nor is the background");
    assert!(!filled(&r, 5, 10), "lines aren't transparent");
    // With nothing enclosed there is nothing to fill.
    let none = lasso_mask(&[(1.0, 1.0), (8.0, 1.0), (8.0, 8.0)], img.area());
    assert!(enclosed_region(&img.px, img.area(), &none, Target::Transparent, &o).is_none());
}

#[test]
fn enclose_and_fill_closes_gaps_and_scales() {
    let img = open_box(3);
    let lasso = lasso_mask(&[(3.0, 3.0), (37.0, 3.0), (37.0, 37.0), (3.0, 37.0)], img.area());
    let leaky = enclosed_region(&img.px, img.area(), &lasso, Target::Transparent, &hard());
    assert!(leaky.is_none_or(|r| !filled(&r, 20, 20)), "the box leaks into the background");
    let o = FillOptions { close_gap: 4.0, scale: 1.0, ..hard() };
    let r = enclosed_region(&img.px, img.area(), &lasso, Target::Transparent, &o).unwrap();
    assert!(filled(&r, 20, 20) && filled(&r, 5, 20), "inside, and grown onto the line");
    assert!(!filled(&r, 4, 20));
}

#[test]
fn white_and_transparent_fills_line_art_on_paper() {
    let mut img = Img::new(30, 30, [255, 255, 255, 255]);
    img.outline(5, 5, 20, 20);
    let lasso = lasso_mask(&[(2.0, 2.0), (25.0, 2.0), (25.0, 25.0), (2.0, 25.0)], img.area());
    let o = hard();
    assert!(enclosed_region(&img.px, img.area(), &lasso, Target::Transparent, &o).is_none(), "paper isn't transparent");
    let r = enclosed_region(&img.px, img.area(), &lasso, Target::WhiteAndTransparent, &o).unwrap();
    assert!(filled(&r, 10, 10) && !filled(&r, 5, 10) && !filled(&r, 3, 3));
}

#[test]
fn paint_unfilled_area_fills_the_small_areas_the_stroke_covers() {
    let mut img = Img::new(40, 20, CLEAR);
    img.outline(5, 5, 9, 9); // a small hole left unfilled
    img.outline(20, 2, 38, 18); // a big area the stroke only crosses
    let stroke = stroke_mask(&[(3.0, 7.0), (25.0, 7.0)], 5.0, img.area());
    assert!(stroke[7 * 40 + 14] && !stroke[0]);
    let r = enclosed_region(&img.px, img.area(), &stroke, Target::Transparent, &hard()).unwrap();
    assert!(filled(&r, 7, 7) && !filled(&r, 25, 7));
}

#[test]
fn anti_aliasing_softens_the_edge() {
    let mut img = Img::new(20, 20, CLEAR);
    img.outline(5, 5, 15, 15);
    let r = img.click(10, 10, FillOptions::default()).unwrap();
    let edge = r.at(6, 10);
    assert!(edge > 0.0 && edge < 1.0, "{edge}");
    assert_eq!(r.at(10, 10), 1.0);
}

#[test]
fn hostile_input_never_panics() {
    let img = open_box(3);
    let weird = FillOptions { tolerance: f32::NAN, close_gap: f32::INFINITY, scale: f32::NEG_INFINITY, ..hard() };
    let _ = img.click(20, 20, weird);
    let _ = img.click(20, 20, FillOptions { close_gap: 1e9, scale: 1e9, ..hard() });
    assert!(img.click(-1, 0, hard()).is_none());
    assert!(img.click(40, 40, hard()).is_none());
    assert!(click_region(&img.px[..10], img.area(), (1, 1), &hard()).is_none(), "the pixels don't cover the area");
    assert!(click_region(&[], Rect::EMPTY, (0, 0), &hard()).is_none());
    assert!(enclosed_region(&img.px, img.area(), &[true; 3], Target::Transparent, &hard()).is_none());
    assert!(stroke_mask(&[(f32::NAN, 1.0), (1e30, 1e30)], f32::NAN, img.area()).iter().all(|v| !v));
    assert!(lasso_mask(&[(0.0, 0.0)], img.area()).iter().all(|v| !v));
    // A whole canvas of one colour, gaps on: nothing to close.
    let blank = Img::new(16, 16, CLEAR);
    let r = blank.click(3, 3, FillOptions { close_gap: 8.0, image_border: true, scale: -3.0, ..hard() }).unwrap();
    assert!(filled(&r, 8, 8));
}

/// A 24 MP canvas of line art with gaps: the fill stays interactive (release build).
#[test]
#[ignore = "timing; run with --release -- --ignored"]
fn big_canvas_timing() {
    let (w, h) = (6000usize, 4000usize);
    let mut img = Img::new(w, h, CLEAR);
    for k in 0..60 {
        let (x0, y0) = (k * 97 % 5000, k * 61 % 3200);
        img.outline(x0 + 10, y0 + 10, x0 + 900, y0 + 700);
        img.set(x0 + 400, y0 + 10, CLEAR);
        img.set(x0 + 401, y0 + 10, CLEAR);
    }
    for (name, o) in [
        ("plain", hard()),
        ("close gap 5", FillOptions { close_gap: 5.0, ..hard() }),
        ("close gap 5, darkest 3, aa", FillOptions { close_gap: 5.0, scale: 3.0, ..FillOptions::default() }),
    ] {
        let t = std::time::Instant::now();
        let r = img.click(3000, 2000, o);
        eprintln!("{name}: {:?} ({:?})", t.elapsed(), r.map(|r| r.bbox));
    }
}

/// The click as it was computed before it worked on the clicked spot's box: everything over the
/// whole page, the seed's area picked from it. (Gap closing reads the seed's spot, as it now
/// does: before, the blank margin past the canvas edge let it go round a line that touches the
/// edge into the next spot.)
fn click_region_whole_page(img: &[[u8; 4]], area: Rect, seed: (i32, i32), o: &FillOptions) -> Option<Region> {
    let (w, h) = dims(img, area)?;
    let si = (seed.1 - area.y0) as usize * w + (seed.0 - area.x0) as usize;
    let target = premul(*img.get(si)?);
    let strength = strengths(img, |p| difference(p, target));
    let margin = margin(o.tolerance);
    let free: Vec<bool> = strength.iter().map(|s| *s <= margin).collect();
    let gap = finite(o.close_gap, MAX_GAP);
    let mut spot = vec![false; free.len()];
    flood(&free, &mut spot, w, h, si, true)?;
    let mut fill = if gap < 1.0 {
        spot
    } else {
        let (labels, seed_label) = areas(&spot, w, h, gap, [o.image_border; 4], Some(si));
        let l = seed_label?;
        let mut fill: Vec<bool> = labels.iter().map(|v| *v == l).collect();
        cover_seams(&mut fill, &labels, &spot, w, h);
        fill
    };
    scale(&mut fill, &strength, &free, w, h, o);
    to_region(&fill, w, h, area, o.anti_alias)
}

/// Random line art: thick strokes (2–4 px, so no area touches another one corner to corner),
/// some broken by gaps, some soft, and a few blobs.
fn random_lines(w: usize, h: usize, seed: u64) -> Img {
    let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut next = move |n: usize| {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as usize) % n.max(1)
    };
    let mut img = Img::new(w, h, CLEAR);
    for _ in 0..14 {
        let (x0, y0, x1, y1) = (next(w) as f32, next(h) as f32, next(w) as f32, next(h) as f32);
        let r = 1.0 + next(3) as f32 * 0.5;
        let alpha = [255u8, 255, 255, 160][next(4)];
        let gap_at = next(100) as f32 / 100.0;
        let gap = [0.0, 0.0, 0.04, 0.08][next(4)];
        let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt().max(1.0);
        let steps = (len * 2.0) as usize;
        for k in 0..=steps {
            let t = k as f32 / steps.max(1) as f32;
            if (t - gap_at).abs() < gap {
                continue;
            }
            let (cx, cy) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
                for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)) <= r * r {
                        img.set(x as usize, y as usize, [0, 0, 0, alpha]);
                    }
                }
            }
        }
    }
    img
}

/// Working on the clicked spot's box gives exactly what working on the whole page gave, for every
/// gap, Area Scaling mode and amount, colour margin and Image Border setting.
#[test]
fn the_spot_box_gives_what_the_whole_page_gave() {
    let mut checked = 0;
    for page in 0..12u64 {
        let img = random_lines(97, 83, page);
        for click in 0..6 {
            let (x, y) = ((page as i32 * 37 + click * 29) % 97, (page as i32 * 11 + click * 41) % 83);
            for gap in [0.0, 3.0, 6.0, 11.0] {
                for (scaling, amount) in [(Scaling::Darkest, 0.0), (Scaling::Darkest, 3.0), (Scaling::Round, 2.0), (Scaling::Square, 2.0), (Scaling::Round, -2.0)] {
                    for border in [false, true] {
                        let o = FillOptions { close_gap: gap, scale: amount, scaling, image_border: border, tolerance: 20.0, ..FillOptions::default() };
                        let got = click_region(&img.px, img.area(), (x, y), &o);
                        let want = click_region_whole_page(&img.px, img.area(), (x, y), &o);
                        assert_eq!(got, want, "page {page}, click ({x}, {y}), gap {gap}, {scaling:?} {amount}, border {border}");
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 2000);
}


/// With Image Border off the canvas carries on blank past its edge, but a fill still never goes
/// round a line that reaches the edge into the next spot (it once did, through that margin).
#[test]
fn a_line_touching_the_edge_keeps_the_spots_apart() {
    let mut img = Img::new(40, 20, CLEAR);
    for y in 0..20 {
        for x in 19..22 {
            img.set(x, y, INK);
        }
    }
    for border in [false, true] {
        let o = FillOptions { close_gap: 3.0, image_border: border, ..hard() };
        let r = click_region(&img.px, img.area(), (5, 10), &o).unwrap();
        assert!(filled(&r, 5, 0) && filled(&r, 18, 19), "border {border}");
        assert!(!filled(&r, 30, 0) && !filled(&r, 30, 10), "border {border}: the fill crossed the line");
    }
}

/// A long white stroke between ink that tapers to a point, its edges jagged as drawn line art
/// is: with gaps closed, a click in its wide end fills it to the point (it isn't a gap: nothing
/// is on the other side), as Clip Studio does. Before, every place where the jagged narrow part
/// briefly widened past the gap became an area of its own, and the fill stopped at the first.
#[test]
fn a_stroke_tapering_to_a_point_fills_to_its_tip() {
    let (w, h) = (60, 260);
    let mut img = Img::new(w, h, INK);
    let mut s = 12345u32;
    let mut jitter = move || {
        s = s.wrapping_mul(1103515245).wrapping_add(12345);
        ((s >> 16) % 3) as f32 - 1.0
    };
    for y in 10..250 {
        let t = (y - 10) as f32 / 240.0;
        let half = 12.0 * (1.0 - t) + 0.6;
        let (l, r) = (30.0 - half + jitter() * 0.8, 30.0 + half + jitter() * 0.8);
        for x in 0..w {
            if (x as f32) >= l && (x as f32) < r {
                img.set(x, y, CLEAR);
            }
        }
    }
    for gap in [3.0, 5.0, 8.0] {
        let o = FillOptions { close_gap: gap, ..hard() };
        let r = click_region(&img.px, img.area(), (30, 20), &o).unwrap();
        let reached = (10..250).rev().find(|&y| (0..w as i32).any(|x| filled(&r, x, y) && img.px[y as usize * w + x as usize] == CLEAR)).unwrap();
        assert!(reached >= 240, "gap {gap}: the fill stopped at row {reached} of a stroke reaching row 249");
    }
}

/// A cell behind a gap in its wall stays apart from the area outside it, as before: only the
/// small bits a jagged narrow part leaves join their neighbour.
#[test]
fn a_real_cell_behind_a_gap_stays_apart() {
    let mut img = Img::new(80, 60, CLEAR);
    // A 26 × 26 box with a 4 px gap in its left wall, in a big open page.
    img.outline(40, 15, 66, 41);
    for y in 26..30 {
        img.set(40, y, CLEAR);
    }
    let o = FillOptions { close_gap: 5.0, ..hard() };
    let outside = click_region(&img.px, img.area(), (10, 10), &o).unwrap();
    assert!(filled(&outside, 10, 50) && !filled(&outside, 53, 28), "the fill stopped at the gap");
    let inside = click_region(&img.px, img.area(), (53, 28), &o).unwrap();
    assert!(filled(&inside, 60, 35) && !filled(&inside, 10, 10));
}

/// Enclose and Fill on the lasso's box and a margin gives what working on the whole page gave,
/// for lassos big and small, at the canvas edge and away from it, every gap, Area Scaling mode and
/// Image Border setting.
#[test]
fn the_lasso_box_gives_what_the_whole_page_gave() {
    let (w, h) = (260usize, 220usize);
    let mut checked = 0;
    let mut cropped = 0;
    for page in 0..10u64 {
        let img = random_lines(w, h, page + 100);
        for k in 0..5i32 {
            // Rectangles and triangles of 20–110 px, some reaching past the canvas edge.
            let (cx, cy) = (((page as i32 * 53 + k * 71) % 280) as f32 - 10.0, ((page as i32 * 31 + k * 47) % 240) as f32 - 10.0);
            let s = 10.0 + ((page as i32 * 13 + k * 29) % 45) as f32;
            let poly: Vec<(f32, f32)> = if k % 2 == 0 {
                vec![(cx - s, cy - s), (cx + s, cy - s), (cx + s, cy + s), (cx - s, cy + s)]
            } else {
                vec![(cx - s, cy + s), (cx, cy - s), (cx + s * 1.2, cy + s * 0.8)]
            };
            let lasso = lasso_mask(&poly, img.area());
            let whole = crate::selection::polygon(&poly, img.area(), false).iter().map(|c| *c >= 0.5).collect::<Vec<bool>>();
            assert_eq!(lasso, whole, "page {page}, lasso {k}: the lasso's box rasterizes as the page did");
            for gap in [0.0, 3.0, 6.0, 11.0] {
                for (scaling, amount) in [(Scaling::Darkest, 0.0), (Scaling::Darkest, 3.0), (Scaling::Round, 2.0), (Scaling::Round, -2.0)] {
                    for border in [false, true] {
                        let o = FillOptions { close_gap: gap, scale: amount, scaling, image_border: border, tolerance: 20.0, ..FillOptions::default() };
                        let got = enclosed_region(&img.px, img.area(), &lasso, Target::Transparent, &o);
                        let want = enclosed_in(&img.px, img.area(), &lasso, w, h, [border; 4], Target::Transparent, &o);
                        assert_eq!(got, want, "page {page}, lasso {k} {poly:?}, gap {gap}, {scaling:?} {amount}, border {border}");
                        checked += 1;
                        let r = (gap + 2.0) / 2.0;
                        let pad = if gap < 1.0 { 2.0 } else { (8.0 * r).ceil() } + amount.abs().ceil() + 4.0;
                        if cx - s * 1.2 - pad > 0.0 || cy - s - pad > 0.0 {
                            cropped += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 1500 && cropped > 500, "{checked} cases, {cropped} cropped");
}
