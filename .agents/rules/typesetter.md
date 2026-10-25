# Rules for PhotoCraft & TypeR Module

1. Strict Rust Guidelines:
   - NEVER use `unwrap()`, `expect()`, or `panic!()` in non-test code (enforced by #![deny(clippy::unwrap_used)]).
   - Always run `cargo check -p photocraft-ui-egui` after editing files.

2. Architecture Constraints:
   - Never introduce WebView, Electron, CEF, or JavaScript dependencies.
   - UI code belongs exclusively in `crates/ui-egui/`.
   - Business logic, presets, and geometry belong in `crates/engine/src/typesetter.rs`.
   - All layer modifications MUST go through `self.run("type.create", ...)` or engine commands.

3. Typography & Text Centering:
   - Point text creation with `align: "center"` must account for vertical baseline offset (`font_size * 0.35`).
   - If `doc.selection` is present, always center text inside `selection.content_bounds()`.