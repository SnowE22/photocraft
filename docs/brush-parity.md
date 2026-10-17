# Brush and painting parity

> **Last reviewed:** 2026-10-17 · **Last updated:** 2026-10-17 · **Change:** minor (Dual Brush, Texture, Wet Edges, spacing and scatter measured against Photoshop 2026 and matched; ABR import reads every Texture/Dual Brush mode spelling; no brush-feel estimate changed) · **Target:** Adobe Photoshop 2026

Painting is where Photoshop users notice differences first, so it gets its own checklist: the
brush engine (`crates/paint/src/brush.rs`), the Brush Settings UI
(`crates/ui-egui/src/brush_sections.rs`), the live stroke renderer, pen input
([`hardware-parity.md`](hardware-parity.md)), and the brush-based tools. Status words as in
[`ui-parity.md`](ui-parity.md).

**Summary (estimated):** breadth ~90% (every Brush Settings section and option exists), ready for
real work ~55%. The engine models what Photoshop models; the gaps are feel (latency, pressure
response, edge quality, smoothing), a few missing options, and live previews for retouch tools.
Remaining: 100–180 h (gap G4 in [`gaps.md`](gaps.md)). Human: feel must be checked on real
tablets against Photoshop side by side.

## Brush Settings sections

| Section | Status | Evidence |
|---|---|---|
| Brush Tip Shape: size, angle, roundness, hardness, spacing, flip X/Y | done | `brush.rs` |
| Spacing is a percentage of the tip's shorter side (a tall, wide or flattened tip spaces by its narrow side; Dual Brush tip too) | done | `TipShape::short_side`; `spacing_follows_the_tips_shorter_side`, measured 2026-10-17 |
| Shape Dynamics (size/angle/roundness jitter, pen pressure/tilt/rotation/wheel controls, minimums, flip jitter, brush projection) | done | #174 |
| Scattering (both axes, count, count jitter) | done | |
| Both Axes scatters in a disc (random direction and distance), tip and Dual Brush tip | done | `disc_offset`; `both_axes_scatter_fills_a_disc`, measured 2026-10-17 |
| Texture (pattern, scale, brightness, contrast, mode, depth, each tip) | done | |
| Dual Brush | done | |
| Dual Brush modes as Photoshop paints them (Hard Mix a ramp, Linear Height an inverted Overlay, Color Dodge/Burn at 248/255 of full depth); each dual dab at a random angle | done | `dual_combine`; `dual_modes_match_photoshop`, `dual_dabs_turn_at_random`, measured 2026-10-17 |
| Stroke masks in Photoshop's order: Texture, then Dual Brush, then Wet Edges, then opacity | done | `BrushContext::stroke_mask`; `dual_brush_comes_before_wet_edges`, `texture_comes_before_the_dual_brush` |
| Dual Brush tip keeps its proportion when the brush is resized (`[` / `]`, size slider, on-canvas resize, a stroke's `size`) | done | `BrushSettings::scale_dual_with_size`; `resizing_the_brush_scales_its_dual_tip` |
| Color Dynamics (FG/BG, hue/sat/brightness jitter, purity, per tip) | done | |
| Hue Jitter uses a standard percentage-amount track instead of a hue spectrum | done | #3825 (Hue Jitter appearance only); rendered track checked in all 14 themes; numeric and pointer edits retain percentage units and engine command routing. |
| Transfer (opacity/flow/wetness/mix jitter and controls) | done | |
| Shape Dynamics and Transfer minimum fields disabled when their Control is Off, retaining their values | done | #3825 (minimum-field interaction only); `brush_sections.rs` UI regressions cover all six fields, reactivation and disabled input |
| Brush Pose (tilt, rotation, pressure overrides) | done | |
| Noise, Wet Edges, Build-up, Protect Texture | done | |
| Wet Edges is Photoshop's curve on the stroke's coverage (half paint × 1.5, solid paint 150/255) | done | `wet_edges`; `wet_edges_curve_matches_photoshop`, measured 2026-10-17 |
| Smoothing: amount, Pulled String, Stroke Catch-up, Catch-up on End, Adjust for Zoom | done | `brush.rs:428-438` |
| Section locks | done | #174 |
| Noise, Wet Edges, Build-up, Smoothing and Protect Texture: checkbox-only rows keep the selected controls open | done | `brush_panel_tests.rs`, #4283; Smoothing enable/disable preserves its amount and options, including imported ABR tool settings |
| Brush tip from an image file | missing | TOOL-213-12 |
| Erodible, Airbrush and Bristle tips (Photoshop's special tips) | partial | not audited in detail; "3D-based brushes don't work" (#1660) |
| Anti-aliasing off for brushes | missing | #1666 (Pencil is the aliased path) |

## Pressure and pen

| Behaviour | Status | Evidence |
|---|---|---|
| Pressure drives size/opacity/flow per sample (no shared pressure within a frame) | done | #2112 |
| Global pressure curve in Preferences | done | #2195 |
| Per-control pressure curves | missing | TOOL-213-11 |
| Quick taps keep their pressure | partial | #1967 fixed one case; tap paints a full blob (#1798), tap sensitivity (#1933) |
| Smooth pressure on Windows | partial | "jumpy" (#2046) |
| Tilt and rotation drive angle/roundness | partial | macOS, X11, web; not Windows; rotation not applied (#1301) |
| Eraser end of the pen switches to Eraser | partial | macOS; flipped Wacom on Windows not (#2285) |

## Stroke quality and feel

| Behaviour | Status | Evidence |
|---|---|---|
| No seams, hard edges stay hard, zero-scatter defaults, ABR tips never black | done | TOOL-213-13 |
| Smooth line weight along a pressure ramp | partial | stair-steps (#1776) |
| Interpolation and smoothing match Photoshop | partial | #2138, #2057 |
| Soft-edge falloff matches Photoshop's curve | partial | #2149, #1619 |
| Brush preview and cursor crisp, not pixelated | partial | #1433 |
| Cursor outline equals the painted footprint at every zoom | partial | #1761, #2364; shaped tips trace their outline (#3467), turned and flipped with a rotated or flipped view |
| Latency: dab to screen within a frame | partial | budgets P13/P16/P33 over (scorecard); big-brush lag #2619, #1994; cursor lag #2566, #1343 |
| Large brushes keep up with the pointer | done | dabs and the stroke composite run tile by tile on every core, only over each tip's footprint (`CoverageMap::apply_dabs`, `paint_rect`, row spans); a 500 px sampled tip at 3 % with a Dual Brush and texture: 4.0 ms a frame dab to screen (P60, RTX 5070 laptop), was ~19 ms before the canvas; a 1500 px tip at 1 %: ~8 ms of engine time a frame, was ~290 ms |
| No hitch at mouse-up on a long stroke | not measurable | P32 needs a frame harness |
| Live stroke for Brush and Eraser | done | |
| Live stroke for every brush-based retouch tool | missing | TOOL-213-7 |
| No OS arrow cursor over the canvas with a pen | partial | TOOL-213-14 |

## Gestures and cursors

| Behaviour | Status | Evidence |
|---|---|---|
| ⌃⌥-drag (macOS) / Alt+right-drag (Windows) resizes and changes hardness with a readout | done | TOOL-213-10 |
| `[` `]` size, `⇧[` `⇧]` hardness, number keys opacity, ⇧+numbers flow | done | 2026-10-07 |
| ⌥-click samples a colour with painting tools; Eyedropper comparison ring | done | TOOL-213-9 |
| ⇧-click draws a straight line from the last point | done | |
| Hold `~` with the Brush or Pencil to erase with its brush: Clear mode while held (rebindable under Tools › Temporary) | done | `hold_keys.rs`, #3439 |
| Clear painting mode in the Brush and Pencil Mode menus | done | `paint_mode_dropdown` in `panels.rs`, `"mode": "clear"`, #3439 |
| Right-click opens the Brush Preset picker | done | [`context-menu-parity.md`](context-menu-parity.md); XP-Pen right-click not (#1980), macOS issue #2360 |
| Precise / normal / full-size tip cursors, crosshair in the tip | partial | Caps Lock precise cursor not audited; round Pencil cursor requested (#2662, #1057) |
| Brush choice kept per tool (Brush, Eraser, Blur each their own) | partial | #2542 |

## Symmetry

| Mode | Status |
|---|---|
| Vertical, Horizontal, Dual axis, Diagonal | done |
| From a path (Make Symmetry Path) | done |
| Wavy, Circle, Spiral, Parallel Lines, Radial, Mandala | missing (#2659) |

## Brush-based tools

| Tool | Status | Notes |
|---|---|---|
| Brush, Pencil (aliased, Auto Erase), Eraser (Brush/Pencil/Block) | done | Pencil 1 px quality #2139 |
| Mixer Brush (Wet, Load, Mix, Flow, Sample All Layers) | partial | "incomplete" (#2247) |
| Clone Stamp (opacity, flow, aligned, sample, overlay) | partial | Clone Source W/H link and Alt target cursor (TOOL-213-8) |
| Pattern Stamp (aligned, impressionist) | done | |
| History Brush | done | Art History Brush missing |
| Spot Healing, Healing | partial | grain not matched on flat colour (#2104), "horrible in general" (#1092), stops responding after switching (#1751) |
| Blur, Sharpen, Smudge (premultiplied, Sample All Layers, spacing) | done | BUG-207-2/3, #2171 |
| Dodge, Burn, Sponge | done | |
| Background Eraser, Magic Eraser | done | |
| Color Replacement | done | |
| Quick Selection (brush-driven) | done | |

## Presets

| Behaviour | Status | Evidence |
|---|---|---|
| .abr import (v1/v2/v6) and export, persistent presets, folders | done | `psd/src/abr.rs` |
| .abr presets keep Capture Brush Size in Preset, Include Tool Settings and the tool blend mode (picking one leaves the tool's size, opacity, flow and mode alone when Photoshop would) | done | `abr_map::captures`; `captured_size_and_tool_settings_follow_the_file` |
| Erodible, Airbrush and Bristle tips | missing | imported as round tips (their `dtips*` / bristle keys are reported) |
| Brushes panel: drag a folder into a folder | done | #2014; `brush.presets.moveItems` |
| Brushes panel: loose brushes at the top level, drops between groups | done | `brush.presets.moveItems` (`into: []`, `before`/`after`) |
| Brushes panel: Shift/Ctrl multi-select of brushes and groups, drag or delete the selection | done | `brushes_tab::click`, `brush.presets.deleteItems` |
| Brushes panel: click selects a group, the chevron opens it, double-click renames | done | `brushes_tab::folder_row` |
| Brushes panel: stroke tiles side by side, thumbnail-size slider, Brush Name/Stroke/Tip menu | done | `brushes_tab::tile`, footer slider, panel menu |
| Brushes panel: Create a New Group from the selection | done | `brush.presets.moveItems {create}` |
| Tool presets (.tpl) | partial | Tool Presets panel exists; .tpl import not found |
| Import 500 brushes without blocking the UI | not run | P24 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | minor | Brushes measured against Photoshop 2026 by scripting it (strokes of computed and real presets, compared pixel by pixel): Dual Brush modes (Hard Mix, Linear Height), Texture Hard Mix and Linear Burn, Color Dodge/Burn at 248/255 of the depth (Texture and Dual Brush), Wet Edges as a coverage curve, the mask order Texture → Dual Brush → Wet Edges → opacity, spacing by the tip's shorter side, Both Axes scatter in a disc, dual dabs at random angles. ABR import reads Photoshop CC's long mode ids and the older four-character codes (`colorBurn`, `hMix`, `lHgt`, …), which fell back to Multiply |
| 2026-10-17 | minor | Large brushes: dabs and the stroke composite run tile by tile on every core over each tip's footprint only (bit-identical to the one-thread path), a dedicated loop for plain sampled tips; P60 measures one frame dab to screen (4.0 ms for a 500 px imported-style brush). The Dual Brush tip scales with the brush size, as in Photoshop. ABR import reads Capture Brush Size in Preset (`useBrushSize`), Include Tool Settings and the tool blend mode, so picking a downloaded brush keeps the tool's size and settings |
| 2026-10-16 | minor | Brushes panel matches Photoshop's: brushes can sit outside groups and drop between groups, Shift/Ctrl multi-select (drag, delete, New Group), a click selects a group and its chevron opens it, stroke-and-name tiles fill each row with a thumbnail-size slider in the footer, Brush Name/Stroke/Tip in the panel menu, Photoshop's darker list well; new `brush.presets.moveItems` and `deleteItems` |
| 2026-10-13 | minor | Stale "missing tool" row corrected against main: Color Replacement (`paint.colorReplacement`) is a live toolbar tool; no brush-feel estimate changed |
| 2026-10-10 | major | First version, from a source audit and the open issues |
| 2026-10-11 | minor | `~` held with the Brush or Pencil erases with that tool's brush, and Clear joins their Mode menus (#3439) |
| 2026-10-12 | minor | Five Brush Settings switch rows toggle without selecting a properties pane; Smoothing has a default-on switch and retains its settings while disabled (#4283). No brush-feel estimate changed. |
| 2026-10-12 | minor | A shaped tip's cursor outline turns and flips with a rotated or flipped view, and follows pixel aspect correction |
| 2026-10-13 | minor | Shape Dynamics and Transfer minimum inputs follow their Control and retain their values while disabled (#3825, partial). No brush-feel estimate changed. |
| 2026-10-13 | minor | Hue Jitter uses the same themed amount slider as other jitter controls (#3825, partial). No brush-feel estimate changed. |
