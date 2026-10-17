//! Frame times of the full UI while an adjustment layer is edited (a user report: dragging a
//! Hue/Saturation slider or a Curves point, and toggling or moving layers with an adjustment layer
//! selected, lagged on a 2400 × 3000 document with many layers).
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example adjust_bench -- [--size 2400x3000] [--layers 30] [--frames 24] [--cpu]
//! ```
//!
//! `BENCH_VERBOSE=1` also prints the app's breakdown of each row's last frame (refresh kind,
//! composite ms, CPU fallback, spans such as the tone histogram). It runs on the discrete GPU
//! when there is one (kittest's own default prefers a software adapter).
//!
//! The app runs offscreen (egui_kittest on wgpu, as the `snapshot` example), so every number is
//! one whole frame: panels, Properties editor, histogram, canvas refresh. `PHOTOCRAFT_GPU_SYNC` is
//! set so the frame waits for the GPU composite. Each row prints the median, min and max frame.

use std::time::Instant;

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Size};
use photocraft_geom::Rect;
use photocraft_ui_egui::{PhotocraftApp, Services};
use rayon::prelude::*;
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// A document like a retouching file: a photo background plus `n` partial, photo-like layers
/// with assorted opacities and blend modes.
fn document(w: u32, h: u32, n: usize) -> Document {
    let mut doc = Document::with_background("bench", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let format = doc.pixel_format();
    let modes = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay, BlendMode::SoftLight, BlendMode::Normal];
    for i in 0..=n {
        // Layer 0 replaces the white background's pixels with a full-canvas photo.
        let (x0, y0, lw, lh) = if i == 0 {
            (0, 0, w, h)
        } else {
            let f = (i * 7919 % 97) as f32 / 97.0;
            let lw = (w as f32 * (0.35 + 0.6 * f)) as u32;
            let lh = (h as f32 * (0.3 + 0.65 * (1.0 - f))) as u32;
            (((w - lw) as f32 * f) as i32, ((h - lh) as f32 * (1.0 - f)) as i32, lw, lh)
        };
        let mut px = vec![0u8; lw as usize * lh as usize * 4];
        px.par_chunks_mut(lw as usize * 4).enumerate().for_each(|(y, row)| {
            for x in 0..lw as usize {
                let (fx, fy) = (x as f32 / lw as f32, y as f32 / lh as f32);
                let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ i.wrapping_mul(83_492_791)) % 997) as f32 / 997.0 - 0.5;
                let ph = i as f32 * 0.7;
                let r = 0.5 + 0.35 * (fx * 6.0 + ph).sin() + n * 0.05;
                let g = 0.45 + 0.3 * (fy * 9.0 + fx * 2.0 + ph).cos() + n * 0.05;
                let b = 0.4 + 0.4 * fy - 0.1 * (fx * 3.0 + ph).sin() + n * 0.05;
                // Soft edges so the layers overlap like cut-outs.
                let edge = (fx.min(1.0 - fx).min(fy).min(1.0 - fy) * 12.0).min(1.0);
                let a = if i == 0 { 1.0 } else { edge };
                for (c, v) in [r, g, b, a].into_iter().enumerate() {
                    row[x * 4 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        });
        let surface = photocraft_raster::Surface::from_interleaved(format, Rect::from_xywh(x0, y0, lw, lh), &px);
        if i == 0 {
            if let Some(bg) = doc.layers.first_mut() {
                bg.content = LayerContent::Raster(surface);
            }
            continue;
        }
        let mut l = Layer::new(format!("Layer {i}"), LayerContent::Raster(surface));
        l.blend = modes[i % modes.len()];
        l.opacity = 0.55 + 0.45 * ((i * 31 % 10) as f32 / 10.0);
        doc.layers.push(l);
    }
    doc
}

struct Bench {
    harness: egui_kittest::Harness<'static, PhotocraftApp>,
}

impl Bench {
    fn app(&mut self) -> &mut PhotocraftApp {
        self.harness.state_mut()
    }

    fn run(&mut self, id: &str, p: Value) -> Value {
        match self.app().run(id, p) {
            Ok(v) => v,
            Err(e) => panic!("{id}: {e}"),
        }
    }

    /// One frame, timed.
    fn frame(&mut self) -> f64 {
        let t = Instant::now();
        self.harness.step();
        t.elapsed().as_secs_f64() * 1000.0
    }

    fn settle(&mut self) {
        for _ in 0..6 {
            self.harness.step();
        }
    }

    fn report(name: &str, mut v: Vec<f64>) {
        v.sort_by(f64::total_cmp);
        let med = v.get(v.len() / 2).copied().unwrap_or(0.0);
        println!(
            "{name:<52} {med:>8.1} ms/frame  (min {:>7.1}, max {:>7.1}, {} frames)",
            v.first().copied().unwrap_or(0.0),
            v.last().copied().unwrap_or(0.0),
            v.len()
        );
    }

    /// A slider or curve drag: `frames` live-preview frames, each with new params from `at`.
    fn drag(&mut self, name: &str, layer: LayerId, frames: usize, at: impl Fn(&Value, usize) -> Value) {
        let base = {
            let st = self.app().session.active().expect("document");
            let LayerContent::Adjustment(a) = &st.doc.layer(layer).expect("layer").content else { panic!("not an adjustment") };
            photocraft_engine::adjust_params::to_params(a)
        };
        let mut times = Vec::new();
        for i in 0..frames {
            self.app().live_adjust = Some((layer, at(&base, i + 1)));
            times.push(self.frame());
        }
        Self::report(name, times);
        self.perf();
        // Release: one commit, as the Properties editor does on drag stop.
        let mut p = at(&base, frames);
        p["layer"] = json!(layer.0);
        let t = Instant::now();
        self.run("layer.setAdjustment", p);
        self.app().live_adjust = None;
        let commit = t.elapsed().as_secs_f64() * 1000.0 + self.frame();
        Self::report(&format!("{name}: release (commit + frame)"), vec![commit]);
        self.perf();
        self.settle();
    }

    /// `frames` repetitions of an edit run through the engine, each followed by one frame.
    fn edits(&mut self, name: &str, frames: usize, mut edit: impl FnMut(&mut Self, usize)) {
        let mut times = Vec::new();
        for i in 0..frames {
            let t = Instant::now();
            edit(self, i);
            times.push(t.elapsed().as_secs_f64() * 1000.0 + self.frame());
        }
        Self::report(name, times);
        self.perf();
        self.settle();
    }

    /// The app's own breakdown of the last frame (`BENCH_VERBOSE`).
    fn perf(&mut self) {
        if std::env::var_os("BENCH_VERBOSE").is_some() {
            let p = &self.app().perf;
            println!(
                "    last refresh {} {:.1} ms composite, {:.1} ms upload, {} px, fallback {:?}, ui {:.1} ms, spans {:?}",
                p.last_refresh, p.composite_ms, p.upload_ms, p.last_refresh_px, p.gpu_fallback, p.ui_ms, p.spans
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, h) =
        arg(&args, "--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(2400), b.parse().unwrap_or(3000)))).unwrap_or((2400, 3000));
    let n: usize = arg(&args, "--layers").and_then(|v| v.parse().ok()).unwrap_or(30);
    let frames: usize = arg(&args, "--frames").and_then(|v| v.parse().ok()).unwrap_or(24).max(1);
    let cpu = args.iter().any(|a| a == "--cpu");
    with_gpu_sync();

    let doc = document(w, h, n);
    let harness = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).wgpu_setup(hardware_setup()).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        // As the desktop app (`PHOTOCRAFT_INLINE_JOBS` unset): long work runs off the UI thread.
        app.background_jobs = true;
        if cpu {
            app.perf.gpu_info.selected = "cpu".into();
            app.perf.gpu_info.canvas = "cpu".into();
        } else if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let info = rs.adapter.get_info();
            println!("adapter: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
            app.set_wgpu(rs.clone());
        }
        app
    });
    let mut b = Bench { harness };
    b.app().session.open_document(doc, None);
    let t = Instant::now();
    b.settle();
    println!(
        "document {w}×{h} ({:.1} MP), {n} layers + background, {} canvas; first frames {:.0} ms",
        (w * h) as f64 / 1e6,
        if cpu { "CPU" } else { "GPU" },
        t.elapsed().as_secs_f64() * 1000.0
    );

    // ---- Hue/Saturation -----------------------------------------------------------------------
    let hue = LayerId(b.run("layer.newAdjustmentLayer.hueSaturation", json!({}))["layer"].as_u64().expect("layer id"));
    b.settle();
    b.drag("Hue/Saturation: drag Hue", hue, frames, |base, i| {
        let mut v = base.clone();
        v["hue"] = json!((i as f64 * 4.0) % 180.0);
        v
    });
    let first = b.app().session.active().expect("document").doc.layers.get(3).map_or(0, |l| l.id.0);
    b.edits("Hue/Saturation selected: toggle a layer's visibility", frames, |b, i| {
        b.run("layer.setProps", json!({"layer": first, "visible": i % 2 == 1}));
    });
    b.run("layer.setProps", json!({"layer": first, "visible": true}));

    // ---- Curves (with its histogram) ------------------------------------------------------------
    let curves = LayerId(b.run("layer.newAdjustmentLayer.curves", json!({}))["layer"].as_u64().expect("layer id"));
    b.settle();
    b.drag("Curves: drag a point", curves, frames, |base, i| {
        let mut v = base.clone();
        v["points"] = json!([[0, 0], [128, (128 + i * 3).min(250)], [255, 255]]);
        v
    });

    // ---- Layers panel edits with the Curves layer selected --------------------------------------
    let ids: Vec<u64> = b.app().session.active().expect("document").doc.layers.iter().map(|l| l.id.0).collect();
    let target = ids.get(3).copied().unwrap_or(ids[0]);
    b.edits("Curves selected: toggle a layer's visibility", frames, |b, i| {
        b.run("layer.setProps", json!({"layer": target, "visible": i % 2 == 1}));
    });
    b.run("layer.setProps", json!({"layer": target, "visible": true}));
    b.settle();
    b.edits("Curves selected: move it down / up", frames, |b, i| {
        b.run(if i % 2 == 0 { "layer.arrange.sendBackward" } else { "layer.arrange.bringForward" }, json!({}));
    });
    b.edits("Curves selected: idle frame", frames, |_, _| {});

    // ---- The same with a pixel layer selected (the baseline) --------------------------------------
    b.run("layer.select", json!({"layer": ids[n / 2]}));
    b.settle();
    b.edits("Pixel layer selected: toggle a layer's visibility", frames, |b, i| {
        b.run("layer.setProps", json!({"layer": target, "visible": i % 2 == 1}));
    });
    b.run("layer.setProps", json!({"layer": target, "visible": true}));

    let (req, _rx) = photocraft_ui_egui::control::ControlRequest::new("ui.inspect", Value::Null);
    let ctx = b.harness.ctx.clone();
    if let photocraft_ui_egui::control::Outcome::Done(v) = photocraft_ui_egui::control::handle(b.harness.state_mut(), &ctx, &req) {
        println!("perf: {}", v["result"]["perf"]["timings"]);
    }
}

/// kittest's default setup prefers a software adapter (deterministic snapshots); time on the
/// hardware the app would use: a discrete GPU, else an integrated one.
fn hardware_setup() -> eframe::egui_wgpu::WgpuSetup {
    use eframe::egui_wgpu::{WgpuSetup, wgpu};
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends.remove(wgpu::Backends::BROWSER_WEBGPU);
    setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, _surface| {
        let mut adapters = adapters.iter().collect::<Vec<_>>();
        adapters.sort_by_key(|a| {
            let info = a.get_info();
            let kind = match info.device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::Other | wgpu::DeviceType::VirtualGpu => 2,
                wgpu::DeviceType::Cpu => 3,
            };
            let backend = match info.backend {
                wgpu::Backend::Metal | wgpu::Backend::Vulkan => 0,
                wgpu::Backend::Dx12 => 1,
                _ => 2,
            };
            (kind, backend)
        });
        adapters.first().map(|a| (*a).clone()).ok_or_else(|| "No adapter found".to_owned())
    }));
    WgpuSetup::CreateNew(setup)
}

/// Make each canvas refresh wait for the GPU so frame times include the composite.
fn with_gpu_sync() {
    // `set_var` is unsafe in edition 2024 and the workspace forbids unsafe code: re-run this
    // binary with the variable set instead.
    if std::env::var_os("PHOTOCRAFT_GPU_SYNC").is_none() {
        let exe = std::env::current_exe().expect("current exe");
        let status = std::process::Command::new(exe).args(std::env::args().skip(1)).env("PHOTOCRAFT_GPU_SYNC", "1").status().expect("re-exec");
        std::process::exit(status.code().unwrap_or(1));
    }
}
