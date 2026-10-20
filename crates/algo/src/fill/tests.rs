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
