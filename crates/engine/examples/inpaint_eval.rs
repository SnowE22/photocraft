//! How well a generative fill rebuilds what it covers: holes cut in a real image are filled and
//! compared with the original (error, sharpness, seam), and side-by-side crops are written.
//!
//! ```sh
//! PHOTOCRAFT_MODEL_DIR=<models> PHOTOCRAFT_ML_DEVICE=cpu \
//!   cargo run --release -p photocraft-engine --example inpaint_eval -- page.jpg out-dir x,y,w,h [x,y,w,h ...]
//! ```
//!
//! Each `x,y,w,h` is an elliptical hole. Variants: `plain` (no refinement, no correction), `lama`
//! (the Remove tool's LaMa fill), `lama-high` (its Quality: High, 1024 crops)
//! and, with `--diffusion`, the diffusion model.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_geom::Rect;
use photocraft_ml::inpaint::{Colours, LAMA, LAMA_QUALITY, Options, Request};
use photocraft_raster::Interrupt;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let diffusion = args.iter().any(|a| a == "--diffusion");
    let args: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (Some(path), Some(out_dir)) = (args.first(), args.get(1)) else {
        eprintln!("usage: inpaint_eval <image> <out-dir> x,y,w,h ...");
        std::process::exit(2);
    };
    let bytes = std::fs::read(path.as_str()).unwrap_or_else(|e| panic!("{path}: {e}"));
    let img = photocraft_codecs::decode(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    let (w, h) = (img.width() as usize, img.height() as usize);
    let rgba = img.to_rgba_f32();
    let rgb: Vec<f32> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    std::fs::create_dir_all(out_dir.as_str()).ok();
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    for (hi, spec) in args.iter().skip(2).enumerate() {
        let v: Vec<i32> = spec.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let [x, y, hw, hh] = v[..] else { panic!("bad hole {spec}") };
        let hole_rect = Rect::new(x, y, x + hw, y + hh).intersect(&canvas);
        let window = hole_rect.inflate(4).intersect(&canvas);
        let (ww, wh) = (window.width() as usize, window.height() as usize);
        let (cx, cy, rx, ry) = (f64::from(x) + f64::from(hw) / 2.0, f64::from(y) + f64::from(hh) / 2.0, f64::from(hw) / 2.0, f64::from(hh) / 2.0);
        let mut hole = vec![false; ww * wh];
        for yy in 0..wh {
            for xx in 0..ww {
                let (px, py) = (f64::from(window.x0) + xx as f64 + 0.5, f64::from(window.y0) + yy as f64 + 0.5);
                hole[yy * ww + xx] = ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0;
            }
        }
        let read = |r: Rect| {
            let mut v = Vec::with_capacity(r.width() as usize * r.height() as usize * 3);
            for yy in r.y0..r.y1 {
                let o = (yy as usize * w + r.x0 as usize) * 3;
                v.extend_from_slice(&rgb[o..o + r.width() as usize * 3]);
            }
            v
        };
        let pixels = read(window);
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false, transparent_unknown: false };
        let mut variants: Vec<(&str, Vec<f32>, f64, usize)> = Vec::new();
        let mut run = |name: &'static str, opts: Options, diff: bool| {
            let t = std::time::Instant::now();
            let mut runs = 0;
            let out = if diff {
                photocraft_ml::diffusion::fill(
                    &req,
                    &mut |r| read(r),
                    &photocraft_ml::diffusion::Settings { prompt: std::env::var("PHOTOCRAFT_EVAL_PROMPT").unwrap_or_default(), ..Default::default() },
                    &Interrupt::NONE,
                )
                .map(|(o, _)| o)
            } else {
                photocraft_ml::inpaint::fill_with(
                    &req,
                    &mut |r| read(r),
                    &mut |a, m, _| {
                        runs += 1;
                        photocraft_ml::lama::inpaint(a, m).map(|(o, _)| o)
                    },
                    &Interrupt::NONE,
                    &opts,
                )
            };
            match out {
                Ok(o) => variants.push((name, o, t.elapsed().as_secs_f64(), runs)),
                Err(e) => eprintln!("{name}: {e}"),
            }
        };
        // `PHOTOCRAFT_EVAL_ONLY_DIFFUSION`: no LaMa (its GPU session stays unloaded).
        if std::env::var_os("PHOTOCRAFT_EVAL_ONLY_DIFFUSION").is_none() {
            run("plain", Options { refine: false, correct: false, ..LAMA }, false);
            run("lama", LAMA, false);
            run("lama-high", LAMA_QUALITY, false);
        }
        if diffusion {
            run("diffusion", LAMA, true);
        }
        // Metrics over the hole.
        let lum = |p: &[f32]| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        let grad = |buf: &[f32], k: usize| {
            let (xx, yy) = (k % ww, k / ww);
            if xx + 1 >= ww || yy + 1 >= wh {
                return 0.0;
            }
            let at = |i: usize| lum(&buf[i * 3..i * 3 + 3]);
            f64::from((at(k + 1) - at(k)).abs() + (at(k + ww) - at(k)).abs())
        };
        let n = hole.iter().filter(|m| **m).count().max(1) as f64;
        let orig_sharp: f64 = hole.iter().enumerate().filter(|(_, m)| **m).map(|(k, _)| grad(&pixels, k)).sum::<f64>() / n;
        println!("hole {spec} ({} px): original sharpness {:.4}", n as u64, orig_sharp);
        for (name, out, secs, runs) in &variants {
            let mae: f64 = hole
                .iter()
                .enumerate()
                .filter(|(_, m)| **m)
                .map(|(k, _)| (0..3).map(|c| f64::from((out[k * 3 + c] - pixels[k * 3 + c]).abs())).sum::<f64>() / 3.0)
                .sum::<f64>()
                / n
                * 255.0;
            let sharp: f64 = hole.iter().enumerate().filter(|(_, m)| **m).map(|(k, _)| grad(out, k)).sum::<f64>() / n;
            // Mean tone in the hole against the original's (a drift shows here).
            let mean = |b: &[f32]| hole.iter().enumerate().filter(|(_, m)| **m).map(|(k, _)| f64::from(lum(&b[k * 3..k * 3 + 3]))).sum::<f64>() / n * 255.0;
            let chroma: f64 = hole
                .iter()
                .enumerate()
                .filter(|(_, m)| **m)
                .map(|(k, _)| {
                    let p = &out[k * 3..k * 3 + 3];
                    f64::from(p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]))
                })
                .sum::<f64>()
                / n
                * 255.0;
            println!(
                "  {name:9} {secs:6.1} s {runs:3} runs  error {mae:5.1}/255  sharpness {sharp:.4} ({:3.0} % of original)  tone {:5.1} (orig {:5.1})  chroma {chroma:4.1}",
                sharp / orig_sharp.max(1e-9) * 100.0,
                mean(out),
                mean(&pixels)
            );
        }
        // Side by side: original, hole, variants.
        let panels = 2 + variants.len();
        let mut sheet = vec![1.0f32; ww * panels * wh * 3];
        let mut put = |panel: usize, buf: &[f32], holed: bool| {
            for yy in 0..wh {
                for xx in 0..ww {
                    let k = yy * ww + xx;
                    let o = (yy * ww * panels + panel * ww + xx) * 3;
                    for c in 0..3 {
                        sheet[o + c] = if holed && hole[k] { if c == 0 { 1.0 } else { 0.3 } } else { buf[k * 3 + c] };
                    }
                }
            }
        };
        put(0, &pixels, false);
        put(1, &pixels, true);
        for (i, (_, out, _, _)) in variants.iter().enumerate() {
            put(2 + i, out, false);
        }
        let image = Image::from_normalized((ww * panels) as u32, wh as u32, ChannelLayout::Rgb, SampleType::U8, &sheet).expect("image");
        let png = photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).expect("png");
        let file = format!("{out_dir}/hole{hi}.png");
        std::fs::write(&file, png).expect("write");
        println!("  -> {file}");
    }
    // `PHOTOCRAFT_EVAL_HOLD_SECS`: stay alive with the models loaded (to read GPU memory).
    if let Some(secs) = std::env::var("PHOTOCRAFT_EVAL_HOLD_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
        println!("holding {secs} s");
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}
