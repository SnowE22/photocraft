# Project Progress Log

## Session 2026-10-20

### Commits Pushed
- `6424a300`: `fix(typesetter): resolve compile errors and add strict development rules`
  - Added `.agents/rules/typesetter.md`
  - Fixed syntax and unwrap rules
- `f50132d3`: `feat(typesetter): recreate TypeR visuals, live preview, script queue, and style catalog`
  - Added page header detection (`Page 1`, `P02`, `Стр. 3`, etc.)
  - Added Line Spacing Sync (`sync_leading_with_size`)
  - Added folder/preset duplication and TypeR JSON export
  - Built Live Preview Card with A-/A+ size buttons
  - Built script queue with page filter and status icons
  - Built folder-tabbed style catalog with inline parameter editor
  - 7 unit tests added in `crates/engine/src/typesetter.rs`
- `e9d4825e`: `fix(typesetter): correct font style, bold/italic, auto-leading and stroke resolution for TypeR import`
  - Resolved font family vs PostScript name lookup bug in `handle_typesetter_action`
  - Supported natural bold/italic faces with `syntheticBold: false`
  - Added `autoLeadingPercentage` calculation for leading
  - Handled stroke object (`enabled`, `size`, `color`, `opacity`) without unwanted 2px default stroke
  - Added `test_import_typer_export_json_sample` and `test_import_real_file_if_present` (all 9 unit tests passing)
- `Phase 3`: `feat(typesetter): dockable interface, auto-persistence, full script window and style buttons grid`
  - Added Photoshop-like panel docking (`Floating`, `DockRight`, `DockLeft`, `DockBottom`) using `egui::Panel`
  - Integrated `typesetter.show(ui)` before `CentralPanel` for proper layout space allocation
  - Implemented automatic persistence to `%APPDATA%/Photocraft/typesetter_state.json` (auto-saves on changes, auto-loads on startup)
  - Added full text script reader view (`TypesetterTab::FullText`) and detached window with pagination, search, read-ahead counter, and quick jump
  - Restructured font styles into compact clickable button grid with dedicated `⚙` settings modal

### Verification Commands
- `cargo test -p photocraft-engine --lib typesetter` (9/9 pass)
- `cargo check -p photocraft-ui-egui` (0 warnings, 0 errors)
- `cargo check -p photocraft` (0 warnings, 0 errors)
