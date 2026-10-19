//! The merged image a ⌘-click on a composite or colour channel loads, rendered ahead while the
//! Channels panel is open (user report: each ⌘-click on a big layered document took about a
//! second, where Photoshop's is instant).
//!
//! The engine keeps the last merged image it rendered while only the selection changes
//! (`channel_cmds::merged`), so the second ⌘-click is quick anyway. This renders it for the first
//! one: once the document has been still for [`SETTLE`] seconds with no mouse button down, on a
//! background thread using half the cores, so a brush stroke started meanwhile still has the
//! rest. A ⌘-click during that render waits for it rather than starting another.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use photocraft_doc::Document;

/// Seconds a document stays unchanged before its merged image is rendered ahead.
const SETTLE: f64 = 0.4;
/// Documents up to this many pixels render quickly enough on a ⌘-click.
const MIN_PIXELS: u64 = 1 << 20;

/// What the panel last saw and the render running.
#[derive(Clone, Default)]
struct Ahead {
    /// The document state last seen and when it was first seen.
    seen: Weak<Document>,
    since: f64,
    /// `seen` needs nothing more: its merged image is kept, or a render of it was started.
    done: bool,
    /// Set by the thread when its render is over.
    running: Option<Arc<AtomicBool>>,
}

fn id() -> egui::Id {
    egui::Id::new("pc-merged-ahead")
}

/// Called each frame the Channels panel shows `doc`: renders its merged image ahead once it has
/// settled (see the module docs).
pub fn prepare(ctx: &egui::Context, doc: &Arc<Document>) {
    if cfg!(target_arch = "wasm32") || doc.size.area() <= MIN_PIXELS {
        return;
    }
    let (now, pressed) = ctx.input(|i| (i.time, i.pointer.any_down()));
    let mut a = ctx.data(|d| d.get_temp::<Ahead>(id())).unwrap_or_default();
    if !a.seen.ptr_eq(&Arc::downgrade(doc)) {
        a = Ahead { seen: Arc::downgrade(doc), since: now, done: false, running: a.running };
    }
    if a.running.as_ref().is_some_and(|r| r.load(Ordering::Acquire)) {
        a.running = None;
    }
    if !a.done && a.running.is_none() {
        if pressed || now - a.since < SETTLE {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLE));
        } else {
            a.done = true;
            if !photocraft_engine::channel_cmds::merged_is_ready(doc) {
                a.running = start(doc.clone());
            }
        }
    }
    ctx.data_mut(|d| d.insert_temp(id(), a));
}

/// Render `doc`'s merged image on a background thread; `None` when no thread could be started.
#[cfg(not(target_arch = "wasm32"))]
fn start(doc: Arc<Document>) -> Option<Arc<AtomicBool>> {
    let over = Arc::new(AtomicBool::new(false));
    let flag = over.clone();
    std::thread::Builder::new()
        .name("merged-ahead".into())
        .spawn(move || {
            // A panic only means the ⌘-click renders it itself.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match pool() {
                Some(p) => p.install(|| photocraft_engine::channel_cmds::prepare_merged(&doc)),
                None => photocraft_engine::channel_cmds::prepare_merged(&doc),
            }));
            flag.store(true, Ordering::Release);
        })
        .ok()?;
    Some(over)
}

#[cfg(target_arch = "wasm32")]
fn start(_: Arc<Document>) -> Option<Arc<AtomicBool>> {
    None
}

/// Half the cores, for renders ahead of need.
#[cfg(not(target_arch = "wasm32"))]
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, |n| n.get());
        rayon::ThreadPoolBuilder::new().num_threads((n / 2).max(1)).thread_name(|i| format!("merged-ahead-{i}")).build().ok()
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_settled_document_gets_its_merged_image_ahead() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 1500, "height": 1000, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 300, "height": 200})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        assert!(!photocraft_engine::channel_cmds::merged_is_ready(&doc));
        let ctx = egui::Context::default();
        // `button`: a press (true) or release (false) in that frame.
        let frame = |t: f64, button: Option<bool>| {
            let mut input = egui::RawInput { time: Some(t), ..Default::default() };
            if let Some(pressed) = button {
                input.events.push(egui::Event::PointerMoved(egui::pos2(5.0, 5.0)));
                input.events.push(egui::Event::PointerButton {
                    pos: egui::pos2(5.0, 5.0),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run_ui(input, |_| prepare(&ctx, &doc));
        };
        let ahead = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<Ahead>(id())).unwrap_or_default();
        frame(0.0, None);
        frame(0.1, None);
        assert!(!ahead(&ctx).done, "not before it has settled");
        // Held down (a stroke): it waits.
        frame(0.5, Some(true));
        frame(0.8, None);
        assert!(!ahead(&ctx).done);
        frame(1.0, Some(false));
        let a = ahead(&ctx);
        assert!(a.done);
        // Other tests share the engine's kept image, so this checks the render ran, not what is kept.
        let running = a.running.expect("a render started");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !running.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline, "the render never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // The same state again: nothing more to do.
        frame(1.5, None);
        let a = ahead(&ctx);
        assert!(a.done && a.running.is_none());
    }
}
