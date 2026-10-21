//! Where saving a document goes: a PSD and a .pcraft export of a file, with the time each tenth
//! of the export's own progress took.
//!
//! ```sh
//! cargo run --release -p photocraft-io --example bench_save -- file.psd
//! ```

use std::sync::Mutex;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("give a file");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let doc = photocraft_io::import(&path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}")).document;
    for ext in ["psd", "pcraft"] {
        let t = Instant::now();
        let marks: Mutex<Vec<(f32, f64)>> = Mutex::new(Vec::new());
        let cancel = || false;
        let progress = |p: f32| {
            let mut m = marks.lock().unwrap();
            if m.last().is_none_or(|(q, _)| p >= q + 0.1 || p >= 1.0 && *q < 1.0) {
                m.push((p, t.elapsed().as_secs_f64() * 1e3));
            }
        };
        let ctl = photocraft_raster::Interrupt::new(&cancel, &progress);
        let r = photocraft_io::export_with(&doc, &format!("copy.{ext}"), &Default::default(), &ctl).unwrap_or_else(|e| panic!("{ext}: {e}"));
        let total = t.elapsed().as_secs_f64() * 1e3;
        let marks: Vec<String> = marks.into_inner().unwrap().iter().map(|(p, ms)| format!("{:.0}%@{ms:.0}", p * 100.0)).collect();
        println!("{ext}: {total:.0} ms, {} MB  [{}]", r.bytes.len() >> 20, marks.join(" "));
    }
}
