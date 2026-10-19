# PhotoCraft & TypeR Architecture Findings

## 1. Engine & UI Architecture
- **Rules (`.agents/rules/typesetter.md`)**:
  - `#![deny(clippy::unwrap_used)]`: Strictly **no** `unwrap()`, `expect()`, or `panic!()` in non-test code.
  - UI strictly belongs in `crates/ui-egui/`. Business logic, presets, geometry in `crates/engine/src/typesetter.rs`.
  - All text creations/edits must pass through `self.run("type.create", ...)` or engine commands.
  - Text optical center baseline offset must be `font_size * 0.35` inside selection bounds.

## 2. Font Resolution Pipeline
- `fontique` matches families by exact or fuzzy family name, not PostScript names.
- If `font` parameter in `type.create` is passed as a PostScript name (e.g. `KuraiVoice2-Bold`), `fontique` fails to find the family and falls back to Arial/Inter.
- `photocraft-text` provides `shared.fonts.resolve_postscript(ps)` which maps a PostScript name to its real family, weight, and italic properties.
- `apply_char_props` accepts:
  - `"font"`: family name (`String`)
  - `"fontStyle"`: style name (`"Regular"`, `"Bold"`, `"Italic"`, `"Bold Italic"`)
  - `"weight"`: numeric weight (100–900, e.g. 700 for Bold)
  - `"italic"`: bool
  - `"fauxBold"`: bool (for synthetic bolding via dilation)
  - `"fauxItalic"`: bool (for synthetic slanting)
  - `"postscriptName"`: exact PostScript name

## 3. TypeR JSON Format Quirks
- **Synthetic vs Natural faces**:
  - `syntheticBold` / `syntheticItalic` is `false` when a font natively possesses that weight or slope.
  - `bold = synthetic_bold || font_style.contains("bold")`
  - `italic = synthetic_italic || font_style.contains("italic") || font_style.contains("oblique")`
- **Stroke representation**:
  - Photoshop CEP TypeR stores stroke as an object:
    `"stroke": {"enabled": bool, "size": number, "opacity": number, "position": "outer", "color": {...}}`
  - If `enabled` is false or `size == 0`, `stroke_width` must be `0.0`.
- **Leading / Auto Leading**:
  - In TypeR exports, `leading` is often omitted in `textStyleRange`. Instead, `autoLeadingPercentage: 1.2` is in `paragraphStyleRange`.
  - Formula: `leading = (font_size * autoLeadingPercentage).round()`.

## 4. Windows rustc Compilation Quirks
- **`STATUS_ACCESS_VIOLATION` (`0xc0000005`)**:
  - Occurs on Windows when `rustc.exe` crashes due to corrupted `target/debug/incremental` cache, file locks from antivirus / background `cargo check` (rust-analyzer), or memory limits during heavy multi-crate parallel compilation.
  - Solution: Re-running `cargo build -p photocraft` or clearing the incremental cache (`Remove-Item -Recurse target\debug\incremental`) resolves it cleanly.

