//! One diffusion run on a square image, from noise or from a hint at several strengths, to see
//! what the starting point does to the fill.
//!
//! ```sh
//! PHOTOCRAFT_MODEL_DIR=<models> cargo run --release -p photocraft-engine --example diffusion_hint_eval -- \
//!   page.png hint.png x,y,w,h out-dir [steps] [strength ...]
//! ```
//!
//! `page.png` and `hint.png` are the same square size (a multiple of 64, the model's side); the
//! rectangle is the hole; a strength of 0 means no hint (from noise).

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

fn planar(path: &str) -> (Vec<f32>, usize) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let img = photocraft_codecs::decode(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    let side = img.width() as usize;
    assert_eq!(side, img.height() as usize, "{path} isn't square");
    let rgba = img.to_rgba_f32();
    let n = side * side;
    let mut out = vec![0f32; 3 * n];
    for (p, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
        for c in 0..3 {
            out[c * n + p] = px[c];
        }
    }
    (out, side)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [page, hint, rect, out_dir, rest @ ..] = &args[..] else {
        eprintln!("usage: diffusion_hint_eval page.png hint.png x,y,w,h out-dir [steps] [strength ...]");
        std::process::exit(2);
    };
    let (rgb, side) = planar(page);
    let (hint, hside) = planar(hint);
    assert_eq!(side, hside);
    let n = side * side;
    let v: Vec<usize> = rect.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let [x, y, w, h] = v[..] else { panic!("bad rect {rect}") };
    let mut mask = vec![0f32; n];
    for yy in y..(y + h).min(side) {
        for xx in x..(x + w).min(side) {
            mask[yy * side + xx] = 1.0;
        }
    }
    let steps: u32 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(25);
    let strengths: Vec<f32> = rest.iter().skip(1).filter_map(|s| s.parse().ok()).collect();
    let strengths = if strengths.is_empty() { vec![0.0] } else { strengths };
    std::fs::create_dir_all(out_dir).ok();
    for s in strengths {
        let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
        // `PHOTOCRAFT_EVAL_GUIDED`: guided steps; `PHOTOCRAFT_EVAL_GUIDANCE`: the scale.
        let settings = photocraft_ml::diffusion::Settings {
            steps,
            strength: s.max(0.05),
            guided_steps: env("PHOTOCRAFT_EVAL_GUIDED").map(|g| g as u32),
            guidance: env("PHOTOCRAFT_EVAL_GUIDANCE").unwrap_or(7.0),
            ..Default::default()
        };
        let t = std::time::Instant::now();
        let r = photocraft_ml::diffusion::inpaint(&rgb, &mask, (s > 0.0).then_some(&hint[..]), side, &settings, &|_| false);
        let (out, device) = match r {
            Ok(o) => o,
            Err(e) => {
                eprintln!("strength {s}: {e}");
                continue;
            }
        };
        println!("strength {s:.2}, {steps} steps: {:.1} s on {device:?}", t.elapsed().as_secs_f32());
        // The hole's rectangle, interleaved.
        let mut crop = Vec::with_capacity(w * h * 3);
        for yy in y..y + h {
            for xx in x..x + w {
                for c in 0..3 {
                    crop.push(out[c * n + yy * side + xx]);
                }
            }
        }
        let image = Image::from_normalized(w as u32, h as u32, ChannelLayout::Rgb, SampleType::U8, &crop).expect("image");
        let png = photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).expect("png");
        let tag = std::env::var("PHOTOCRAFT_EVAL_TAG").unwrap_or_default();
        std::fs::write(format!("{out_dir}/s{:03}-{steps}{tag}.png", (s * 100.0).round() as u32), png).expect("write");
    }
}
