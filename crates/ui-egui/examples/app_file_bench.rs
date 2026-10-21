//! Frame times of the whole app on a real file: the real `PhotocraftApp` in an offscreen
//! egui_kittest harness on wgpu (the GPU canvas, the device the app requests), driven by real
//! pointer and key input: idle and hover frames, selecting and hiding layers, brush strokes,
//! Move drags of the biggest and of a smaller layer, Fill tool clicks, Undo. Each row is the
//! median / p90 / max of a frame's time; `PHOTOCRAFT_GPU_SYNC` makes the canvas wait for the GPU,
//! so a frame's time includes its composite.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example app_file_bench -- file.psd [--reps 9] [--size 1920x1080] [--spin idle|brush|move|select] [--cold|--warm-only|--sleep-only]
//! ```
//!
//! The file opens as in the desktop app: the GPU compositor warming up at start
//! (`PhotocraftApp::warm_up_gpu`) and the file a few seconds later. `--cold` opens it at once
//! without the warm-up; `--warm-only` skips the pause, `--sleep-only` the warm-up (an idle laptop
//! GPU clocks down: the first composite after the pause takes ~0.8 s instead of ~0.3 s).

use std::time::Instant;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use photocraft_doc::LayerId;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

type H = egui_kittest::Harness<'static, PhotocraftApp>;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn frame(h: &mut H) -> f64 {
    let t = Instant::now();
    h.step();
    ms(t)
}

fn settle(h: &mut H, n: usize) -> f64 {
    (0..n).map(|_| frame(h)).fold(0.0, f64::max)
}

fn row(name: &str, v: Vec<f64>) {
    let mut v: Vec<f64> = v.into_iter().filter(|t| t.is_finite()).collect();
    if v.is_empty() {
        return;
    }
    v.sort_by(f64::total_cmp);
    let (med, p90, max) = (v[v.len() / 2], v[(v.len() * 9 / 10).min(v.len() - 1)], v[v.len() - 1]);
    let flag = if med > 16.0 { "  <-- over 16 ms" } else { "" };
    println!("{name:<46} median {med:>8.2} ms   p90 {p90:>8.2}   max {max:>8.2}{flag}");
}

fn screen(h: &H, x: f64, y: f64) -> Pos2 {
    let app = h.state();
    let idx = app.session.active_index().unwrap_or(0);
    let v = &app.ui.views[idx];
    let rect = photocraft_ui_egui::rulers::content_rect(app, app.last_canvas_rect);
    ViewXform { rect, zoom: v.zoom, center: v.center, flip: app.ui.view.flip_horizontal, rotation: v.rotation, aspect: 1.0 }.to_screen(x as f32, y as f32)
}

/// A drag on the canvas: hover, press, `steps` moves, release, one more frame (their times).
fn drag(h: &mut H, from: [f64; 2], d: [f64; 2], steps: usize) -> Vec<f64> {
    let p0 = screen(h, from[0], from[1]);
    let mut times = Vec::new();
    h.event(Event::PointerMoved(p0));
    times.push(frame(h));
    h.event(Event::PointerButton { pos: p0, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    times.push(frame(h));
    for i in 1..=steps {
        let f = i as f64 / steps as f64;
        h.event(Event::PointerMoved(screen(h, from[0] + d[0] * f, from[1] + d[1] * f)));
        times.push(frame(h));
    }
    let p1 = screen(h, from[0] + d[0], from[1] + d[1]);
    h.event(Event::PointerButton { pos: p1, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    times.push(frame(h));
    times.push(frame(h));
    times
}

/// Rows for drags from [`drag`]: the press, the moves, the release.
fn drag_rows(label: &str, drags: &[Vec<f64>]) {
    let pick = |f: &dyn Fn(&[f64]) -> Vec<f64>| drags.iter().flat_map(|d| f(d)).collect::<Vec<f64>>();
    row(&format!("{label}: press"), pick(&|d| d.get(1).copied().into_iter().collect()));
    row(&format!("{label}: drag frames"), pick(&|d| d.get(2..d.len().saturating_sub(2)).unwrap_or_default().to_vec()));
    row(&format!("{label}: release"), pick(&|d| d.len().checked_sub(2).and_then(|i| d.get(i)).copied().into_iter().collect()));
}

fn select(h: &mut H, id: LayerId) {
    let _ = h.state_mut().run("layer.select", json!({"layer": id.0}));
}

fn main() {
    reexec_with_gpu_sync();
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("give a file");
    let reps: usize = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(9).max(3);
    let (sw, sh) = arg(&args, "--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((1920.0, 1080.0));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let t = Instant::now();
    let doc = photocraft_io::import(&path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}")).document;
    println!("import {:.0} ms: {} × {}, {} layers", ms(t), doc.size.width, doc.size.height, doc.walk().len());
    let canvas = doc.bounds();
    let mut pixel: Vec<(i64, LayerId)> = doc
        .walk()
        .iter()
        .filter(|(path, _, l)| doc.shown_at(path) && matches!(l.content, photocraft_doc::LayerContent::Raster(_)))
        .map(|(_, _, l)| {
            let b = photocraft_compose::change_bounds(l, canvas).unwrap_or(canvas);
            (i64::from(b.width()) * i64::from(b.height()), l.id)
        })
        .collect();
    pixel.sort();
    let biggest = pixel.last().map(|p| p.1).expect("a pixel layer");
    let small = pixel.get(pixel.len() / 3).map(|p| p.1).expect("a pixel layer");
    let picks: Vec<LayerId> = pixel.iter().step_by((pixel.len() / 8).max(1)).map(|p| p.1).collect();
    let (w, hh) = (f64::from(doc.size.width), f64::from(doc.size.height));

    // As the desktop app starts: the GPU canvas, its compositor warming up; the file opened a
    // moment later (`--cold`: at once, without the warm-up).
    let cold = args.iter().any(|a| a == "--cold" || a == "--sleep-only");
    let pause = !args.iter().any(|a| a == "--cold" || a == "--warm-only");
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(sw, sh)).with_pixels_per_point(1.0);
    let mut h: H = builder.wgpu_setup(photocraft_ui_egui::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            app.set_wgpu(rs.clone());
            if !cold {
                app.warm_up_gpu();
            }
        }
        app
    });
    settle(&mut h, 3);
    if pause {
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
    let t = Instant::now();
    h.state_mut().session.open_document(doc, Some(path.clone()));
    h.state_mut().sync_views();
    settle(&mut h, 6);
    println!("open + first frames: {:.0} ms (gpu {}, first composite {:.0} ms)", ms(t), h.state().perf.gpu, h.state().perf.composite_ms);
    settle(&mut h, 6);

    if let Some(what) = arg(&args, "--spin") {
        spin(&mut h, &what, biggest, &picks, [w, hh]);
        return;
    }

    row("idle frame", (0..reps).map(|_| frame(&mut h)).collect());
    h.state_mut().ui.tool = Tool::Brush;
    settle(&mut h, 3);
    row(
        "hover over the canvas (brush)",
        (0..reps)
            .map(|i| {
                h.event(Event::PointerMoved(screen(&h, w * 0.4 + i as f64 * 5.0, hh * 0.4)));
                frame(&mut h)
            })
            .collect(),
    );
    let mut k = 0;
    row(
        "select a layer",
        (0..reps)
            .map(|_| {
                k += 1;
                let t = Instant::now();
                select(&mut h, picks[k % picks.len()]);
                (ms(t) + frame(&mut h)).max(frame(&mut h))
            })
            .collect(),
    );
    let mut vis = false;
    row(
        "hide / show a layer",
        (0..reps)
            .map(|_| {
                vis = !vis;
                let t = Instant::now();
                let _ = h.state_mut().run("layer.setProps", json!({"layer": small.0, "visible": vis}));
                (ms(t) + frame(&mut h)).max(frame(&mut h))
            })
            .collect(),
    );
    let _ = h.state_mut().run("layer.setProps", json!({"layer": small.0, "visible": true}));
    settle(&mut h, 3);

    // Brush strokes on a new layer above the middle of the stack.
    select(&mut h, small);
    let _ = h.state_mut().run("layer.new.layer", json!({}));
    h.state_mut().ui.tool = Tool::Brush;
    settle(&mut h, 3);
    let strokes: Vec<Vec<f64>> = (0..reps).map(|i| drag(&mut h, [w * 0.3, hh * 0.3 + i as f64 * 20.0], [w * 0.3, hh * 0.1], 24)).collect();
    drag_rows("brush stroke", &strokes);
    row(
        "undo (Ctrl+Z)",
        (0..reps)
            .map(|_| {
                h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
                frame(&mut h).max(frame(&mut h))
            })
            .collect(),
    );

    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    for (label, id) in [("Move the biggest layer", biggest), ("Move a smaller layer", small)] {
        select(&mut h, id);
        settle(&mut h, 3);
        let drags: Vec<Vec<f64>> = (0..reps).map(|i| drag(&mut h, [w * 0.5, hh * 0.5], if i.is_multiple_of(2) { [37.0, 23.0] } else { [-37.0, -23.0] }, 10)).collect();
        drag_rows(label, &drags);
    }

    // Fill tool clicks on a new layer, referring to all layers.
    select(&mut h, small);
    let _ = h.state_mut().run("layer.new.layer", json!({}));
    h.state_mut().ui.tool = Tool::Fill;
    settle(&mut h, 3);
    let fills: Vec<Vec<f64>> = (0..reps).map(|i| drag(&mut h, [w * (0.3 + 0.05 * i as f64), hh * 0.5], [0.0, 0.0], 1)).collect();
    drag_rows("Fill tool click", &fills);

    println!("perf spans: {:?}", h.state().perf.spans);
}

fn spin(h: &mut H, what: &str, biggest: LayerId, picks: &[LayerId], [w, hh]: [f64; 2]) {
    let t = Instant::now();
    let mut k = 0usize;
    match what {
        "brush" => h.state_mut().ui.tool = Tool::Brush,
        "move" => {
            h.state_mut().ui.tool = Tool::Move;
            h.state_mut().ui.tool_options.move_auto_select = false;
            select(h, biggest);
        }
        _ => {}
    }
    settle(h, 3);
    while t.elapsed().as_secs() < 20 {
        k += 1;
        match what {
            "brush" => {
                drag(h, [w * 0.3, hh * 0.3], [w * 0.3, hh * 0.1], 40);
                let _ = h.state_mut().run("edit.undo", json!({}));
            }
            "move" => {
                let s = if k.is_multiple_of(2) { 1.0 } else { -1.0 };
                drag(h, [w * 0.5, hh * 0.5], [30.0 * s, 20.0 * s], 30);
            }
            "select" => {
                select(h, picks[k % picks.len()]);
                frame(h);
            }
            _ => {
                frame(h);
            }
        }
    }
}

/// Run again with `PHOTOCRAFT_GPU_SYNC=1` unless it is set.
fn reexec_with_gpu_sync() {
    if std::env::var_os("PHOTOCRAFT_GPU_SYNC").is_some() {
        return;
    }
    let exe = std::env::current_exe().expect("exe");
    let status = std::process::Command::new(exe).args(std::env::args().skip(1)).env("PHOTOCRAFT_GPU_SYNC", "1").status().expect("re-exec");
    std::process::exit(status.code().unwrap_or(1));
}
