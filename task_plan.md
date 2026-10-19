# PhotoCraft & TypeR Module Task Plan

## Goal
Implement and maintain full feature and visual parity with TypeR (Photoshop CEP extension) inside PhotoCraft, following strict Rust architecture guidelines (`#![deny(clippy::unwrap_used)]`, no WebViews, native egui UI, modular engine logic).

## Principles & Active Skills
- **`verify-typesetter`**: Continuous automated validation via `cargo test -p photocraft-engine --lib typesetter` and `cargo check -p photocraft-ui-egui`.
- **`impeccable`**: Professional UI/UX standards in "Operate" mode: scanability, precise hierarchy, native look-and-feel, no visual clutter.
- **`planning-with-files`**: Persistent working memory on disk (`task_plan.md`, `findings.md`, `progress.md`).
- **`graphify`**: Deep code navigation and cross-module relationship awareness.

## Phases

### Phase 1: Core TypeR Parity (COMPLETED)
- [x] Page grouping & auto header detection (`Page 1`, `P02`, etc.) in script loader
- [x] Line Spacing Sync (`sync_leading_with_size`)
- [x] Folder & preset duplication in engine
- [x] TypeR CEP JSON export (`export_styles_json`)
- [x] Recreate TypeR UI in egui: Live Preview card, A-/A+ buttons, queue list, styles catalog, folder tabs, settings panel
- [x] Push commits `6424a300` and `f50132d3`

### Phase 2: Font Style & TypeR Import Accuracy (COMPLETED)
- [x] Fix font family vs PostScript name lookup in `handle_typesetter_action`
- [x] Support natural Bold/Italic faces when `syntheticBold` / `syntheticItalic` is `false`
- [x] Support `autoLeadingPercentage` for leading calculation
- [x] Parse stroke objects (`enabled`, `size`, `color`, `opacity`) without defaulting to unwanted 2.0px stroke
- [x] Pass real family, `fontStyle`, `weight`, `fauxBold`, `fauxItalic`, and `postscriptName` to `type.create`
- [x] Commit & push `e9d4825e`

### Phase 3: UX & Workflow Enhancements (UPCOMING / IN PROGRESS)
- [ ] Font picker integration / dropdown with system fonts in style editor
- [ ] Direct text replacement / queue advancement hotkeys (Ctrl+Enter / F4)
- [ ] Batch bubble assignment and selection bounds auto-positioning verification
- [ ] Localization and i18n support for typesetter UI (RU / EN)
