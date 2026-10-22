//! One Nano Banana removal on an image file, with the saved API key (paid: one image per run,
//! about $0.03 for a page up to 1100 px). Writes the result next to the given output path, and
//! the image as sent (`<out>.sent.png`) to see what the model was given.
//!
//! ```sh
//! cargo run -p photocraft-ml --example nano_banana_eval -- page.png x,y,w,h[;x,y,w,h…] out.png ["prompt"]
//! ```

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_geom::Rect;
use photocraft_ml::inpaint::{Colours, Request};
use photocraft_raster::Interrupt;

fn save(path: &str, rgb: &[f32], w: u32, h: u32) {
    let img = Image::from_normalized(w, h, ChannelLayout::Rgb, SampleType::U8, rgb).expect("image");
    std::fs::write(path, photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).expect("png")).expect("write");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [page, rects, out, rest @ ..] = &args[..] else {
        eprintln!("usage: nano_banana_eval page.png x,y,w,h[;x,y,w,h…] out.png [prompt]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(page).unwrap_or_else(|e| panic!("{page}: {e}"));
    let img = photocraft_codecs::decode(&bytes).unwrap_or_else(|e| panic!("{page}: {e}"));
    let (w, h) = (img.width() as usize, img.height() as usize);
    let rgb: Vec<f32> = img.to_rgba_f32().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut hole = vec![false; w * h];
    for r in rects.split(';') {
        let v: Vec<usize> = r.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let [x, y, rw, rh] = v[..] else { panic!("bad rect {r}") };
        for yy in y..(y + rh).min(h) {
            for xx in x..(x + rw).min(w) {
                hole[yy * w + xx] = true;
            }
        }
    }
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    let req =
        Request { canvas, window: canvas, hole: &hole, pixels: &rgb, ch: 3, colours: Colours::Rgb, alpha: None, linear: false, transparent_unknown: false };
    let mut read = |r: Rect| {
        let mut o = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = (y as usize * w + x as usize) * 3;
                o.extend_from_slice(&rgb[k..k + 3]);
            }
        }
        o
    };
    let sent = format!("{out}.sent.png");
    let t = std::time::Instant::now();
    match photocraft_ml::gemini::eval_fill(&req, &mut read, rest.first().map_or("", String::as_str), &Interrupt::NONE, &sent) {
        Ok(filled) => {
            println!("filled in {:.1} s", t.elapsed().as_secs_f32());
            save(out, &filled, w as u32, h as u32);
        }
        Err(e) => println!("error after {:.1} s: {e}", t.elapsed().as_secs_f32()),
    }
}
