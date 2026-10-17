//! Window › Brushes: Photoshop's Brushes panel. The presets form one tree: loose presets and
//! groups at the top level, presets and nested folders inside a group. Each preset is a tile with
//! its stroke and name (and, from the panel menu, its tip); the footer slider sets the tiles'
//! width, and as many as fit share a row.
//!
//! Clicking a preset picks it and selects it; Shift-click selects a range and Ctrl-click (⌘ on
//! macOS) adds or removes one item, as in Photoshop. Clicking a group's row selects the group,
//! its chevron opens and closes it, and double-clicking it renames it. Dragging moves the
//! selection (or the item under the pointer): onto the first or second half of a tile to land
//! before or after it, onto a group row's upper edge (or a closed group's lower edge) to land
//! before (after) the group, onto the rest of the row to go inside it, or into the empty space
//! under the list to go to the end of the top level. Every change is a `brush.presets.*` command
//! (`moveItems` and `deleteItems` for a selection), so it is journaled, drivable, and persisted
//! by the preset store.
//!
//! The list itself ([`preset_list`]) is shared with the Brush Preset picker (`brush_picker`),
//! which draws its own cards; each keeps its own view state.

use std::cell::RefCell;
use std::collections::HashSet;

use egui::{Color32, RichText, Sense, Stroke, pos2, vec2};
use photocraft_engine::brush_preset_items_cmds::{panel_order, place, place_part, under};
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::brush_panel::{BrushesPanelState, Renaming, WIDTH, commit_gesture, full_uv, run_or_status};
use crate::brush_preview;
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

/// A preset or folder in the panel's tree: what is selected, dragged and dropped on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushItem {
    /// A preset, by name.
    Preset(String),
    /// A group (`["Group"]`) or nested folder (`["Group", "Folder"]`), by its path from the top.
    Folder(Vec<String>),
}

impl BrushItem {
    /// The item as `brush.presets.moveItems`/`deleteItems` take it.
    fn param(&self) -> Value {
        match self {
            BrushItem::Preset(n) => json!({ "preset": n }),
            BrushItem::Folder(f) => json!({ "folder": f }),
        }
    }

    /// The item as a sibling to land `before` or `after` (a folder by its own name).
    fn sibling(&self) -> Value {
        match self {
            BrushItem::Preset(n) => json!({ "preset": n }),
            BrushItem::Folder(f) => json!({ "folder": f.last().cloned().unwrap_or_default() }),
        }
    }

    /// The name shown for it.
    pub fn label(&self) -> &str {
        match self {
            BrushItem::Preset(n) => n,
            BrushItem::Folder(f) => f.last().map_or("", String::as_str),
        }
    }

    /// Does the item hold (or is it) preset `p`?
    fn covers(&self, p: &BrushPreset) -> bool {
        match self {
            BrushItem::Preset(n) => p.name == *n,
            BrushItem::Folder(f) => under(p, f),
        }
    }
}

/// What a drag in the Brushes panel carries: the items being moved.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushDrag(pub Vec<BrushItem>);

/// What a preset list does after drawing (the presets are borrowed while it draws).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A click: make the preset current.
    Select(String),
    /// A double-click in the picker: make the preset current and dismiss the picker (Photoshop).
    Choose(String),
    /// View only: a click on a preset or folder. `shift` selects the range from the anchor,
    /// `toggle` (Ctrl, ⌘ on macOS) adds or removes the item; both add the range.
    Click { item: BrushItem, shift: bool, toggle: bool },
    /// View only: a click on the empty list clears the selection.
    Deselect,
    /// View only: open or close a group or folder (by its [`folder_view_key`]; [`preset_list`]
    /// applies it to its view state).
    ToggleGroup(String),
    /// Move items into folder `into` (`[]` = the top level): before or after a sibling there, or
    /// to its end. `create` makes `into`, a new folder where the first item was.
    MoveItems { items: Vec<BrushItem>, into: Vec<String>, before: Option<BrushItem>, after: Option<BrushItem>, create: bool },
    /// View only: show the rename bar for a preset or group ([`preset_list`] applies it).
    BeginRename(Renaming),
    /// Commit a rename typed in the rename bar.
    Rename(Renaming),
    Delete(String),
    /// Delete a group (`folder` empty) or one of its nested folders, with their presets.
    DeleteGroup { group: String, folder: Vec<String> },
    /// Delete presets and folders (with their presets) in one command.
    DeleteItems(Vec<BrushItem>),
}

/// How a preset list is laid out: the Brushes panel's tiles, or the Brush Preset picker's cards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListLayout {
    /// The scroll area's id and its height limit.
    pub id: &'static str,
    pub max_height: f32,
    /// Left margin of the top level (tiles and group rows); each folder level adds
    /// [`FOLDER_INDENT`].
    pub indent: f32,
    /// The Brush Preset picker's cards: a tip cell and a stroke cell on top and the name across
    /// the bottom (or, with only the tip and the name on, in the stroke's cell). Otherwise the
    /// panel's tiles.
    pub cards: bool,
}

/// The Brushes panel's list.
pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0, indent: 4.0, cards: false };

/// Deepest nested folder (inside a group) the panel draws; deeper folders show at this depth.
pub const MAX_VIEW_DEPTH: usize = photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;
/// Longest folder path the panel draws: a group and [`MAX_VIEW_DEPTH`] folders.
const MAX_VIEW_PATH: usize = MAX_VIEW_DEPTH + 1;

/// The collapse key of the group or folder at `path`.
pub fn folder_view_key(path: &[String]) -> String {
    path.join("/")
}

/// A level of the tree in panel order: its presets and folders.
#[derive(Debug)]
pub enum Node<'a> {
    Preset(&'a BrushPreset),
    Folder(FolderNode<'a>),
}

/// A group or nested folder and what it holds.
#[derive(Debug)]
pub struct FolderNode<'a> {
    /// Its path from the top (`["Group"]`, `["Group", "Folder"]`).
    pub path: Vec<String>,
    pub children: Vec<Node<'a>>,
    /// Presets in it and below it.
    pub count: usize,
}

/// The panel's tree of `items` (in library order): loose presets and groups at the top, each
/// folder where its first preset is, presets in their order. Places deeper than
/// [`MAX_VIEW_PATH`] are cut there.
pub fn folder_tree<'a>(items: &[&'a BrushPreset]) -> Vec<Node<'a>> {
    level(items.to_vec(), &[])
}

/// One level of [`folder_tree`] under `prefix`. The recursion is at most [`MAX_VIEW_PATH`] deep.
fn level<'a>(items: Vec<&'a BrushPreset>, prefix: &[String]) -> Vec<Node<'a>> {
    enum Slot {
        Preset(usize),
        Folder(usize),
    }
    let depth = prefix.len();
    let mut slots = Vec::new();
    let mut folders: Vec<(&'a str, Vec<&'a BrushPreset>)> = Vec::new();
    for (i, &p) in items.iter().enumerate() {
        let part = if depth < MAX_VIEW_PATH { place_part(p, depth) } else { None };
        match part {
            None => slots.push(Slot::Preset(i)),
            Some(name) => match folders.iter().position(|(n, _)| *n == name) {
                Some(k) => {
                    if let Some((_, v)) = folders.get_mut(k) {
                        v.push(p);
                    }
                }
                None => {
                    slots.push(Slot::Folder(folders.len()));
                    folders.push((name, vec![p]));
                }
            },
        }
    }
    let mut folders: Vec<Option<(&str, Vec<&'a BrushPreset>)>> = folders.into_iter().map(Some).collect();
    slots
        .into_iter()
        .filter_map(|s| match s {
            Slot::Preset(i) => items.get(i).map(|p| Node::Preset(p)),
            Slot::Folder(k) => {
                let (name, sub) = folders.get_mut(k)?.take()?;
                let path: Vec<String> = prefix.iter().cloned().chain(std::iter::once(name.to_string())).collect();
                let count = sub.len();
                let children = level(sub, &path);
                Some(Node::Folder(FolderNode { path, children, count }))
            }
        })
        .collect()
}

/// Where on an item a drop lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Before,
    After,
    /// Inside a folder, at its end.
    Into,
}

/// What dropping `dragged` on `target` (which sits directly in folder `parent`) does, as in
/// Photoshop's panel: land before or after it there, or go into it (a folder). `None` when the
/// drop does nothing or can't be done: an item onto itself, a folder into itself or into one of
/// its own folders, or into a preset.
pub fn drop_action(dragged: &[BrushItem], target: &BrushItem, parent: &[String], spot: Spot) -> Option<Action> {
    if dragged.is_empty() {
        return None;
    }
    let into: Vec<String> = match (spot, target) {
        (Spot::Into, BrushItem::Folder(f)) => f.clone(),
        (Spot::Into, BrushItem::Preset(_)) => return None,
        _ => parent.to_vec(),
    };
    if dragged.iter().any(|it| matches!(it, BrushItem::Folder(f) if into.starts_with(f))) {
        return None;
    }
    if spot != Spot::Into && dragged.len() == 1 && dragged.first() == Some(target) {
        return None;
    }
    let (before, after) = match spot {
        Spot::Before => (Some(target.clone()), None),
        Spot::After => (None, Some(target.clone())),
        Spot::Into => (None, None),
    };
    Some(Action::MoveItems { items: dragged.to_vec(), into, before, after, create: false })
}

/// Photoshop's Create a New Group: the selected items go into a new group (a folder, when the
/// first of them sits in one) made where the first of them was, and its name opens for editing.
/// `None` with nothing selected.
pub fn new_group(presets: &[BrushPreset], selected: &[BrushItem]) -> Option<(Action, Renaming)> {
    // The first selected preset in panel order, and the outermost selected item holding it.
    let first = panel_order(presets).into_iter().filter_map(|i| presets.get(i)).find(|p| selected.iter().any(|it| it.covers(p)))?;
    let outer = selected.iter().filter(|it| it.covers(first)).min_by_key(|it| match it {
        BrushItem::Folder(f) => f.len(),
        BrushItem::Preset(_) => usize::MAX,
    })?;
    let parent: Vec<String> = match outer {
        BrushItem::Preset(_) => place(first),
        BrushItem::Folder(f) => f.split_last().map(|(_, p)| p.to_vec()).unwrap_or_default(),
    };
    let taken = |name: &str| {
        let mut path = parent.clone();
        path.push(name.to_string());
        presets.iter().any(|p| under(p, &path))
    };
    let name = (1..100_000).map(|n| format!("Group {n}")).find(|n| !taken(n))?;
    let mut into = parent;
    into.push(name);
    let (group, folder) = into.split_first().map(|(g, f)| (g.clone(), f.to_vec()))?;
    let renaming = Renaming { group: true, name: group, folder, text: String::new() };
    Some((Action::MoveItems { items: selected.to_vec(), into, before: None, after: None, create: true }, renaming))
}

/// Apply the view-only actions to `st` (`visible` is the items in the order drawn, for Shift
/// ranges); returns the rest, the commands for [`apply`].
fn view_actions(st: &mut BrushesPanelState, acts: Vec<Action>, visible: &[BrushItem]) -> Vec<Action> {
    acts.into_iter()
        .filter_map(|a| match a {
            Action::ToggleGroup(g) => {
                let c = &mut st.collapsed;
                match c.iter().position(|x| *x == g) {
                    Some(i) => drop(c.remove(i)),
                    None => c.push(g),
                }
                None
            }
            Action::BeginRename(r) => {
                st.renaming = Some(r);
                None
            }
            Action::Click { item, shift, toggle } => {
                click(st, item, shift, toggle, visible);
                None
            }
            Action::Deselect => {
                st.selected.clear();
                None
            }
            a => Some(a),
        })
        .collect()
}

/// A selection click, as in Photoshop's panels: a plain click selects only `item`; Ctrl
/// (`toggle`) adds or removes it; Shift selects from the anchor to it in the drawn order
/// (`visible`), adding to the selection with Ctrl too.
pub fn click(st: &mut BrushesPanelState, item: BrushItem, shift: bool, toggle: bool, visible: &[BrushItem]) {
    if shift {
        let anchor = st.anchor.clone().unwrap_or_else(|| item.clone());
        let range = match (visible.iter().position(|v| *v == anchor), visible.iter().position(|v| *v == item)) {
            (Some(a), Some(b)) => visible.get(a.min(b)..=a.max(b)).map(<[BrushItem]>::to_vec).unwrap_or_default(),
            _ => vec![item.clone()],
        };
        if toggle {
            for r in range {
                if !st.selected.contains(&r) {
                    st.selected.push(r);
                }
            }
        } else {
            st.selected = range;
        }
        if st.anchor.is_none() {
            st.anchor = Some(item);
        }
    } else if toggle {
        match st.selected.iter().position(|s| *s == item) {
            Some(i) => drop(st.selected.remove(i)),
            None => st.selected.push(item.clone()),
        }
        st.anchor = Some(item);
    } else {
        st.selected = vec![item.clone()];
        st.anchor = Some(item);
    }
}

/// Turn a preset list's actions into commands.
pub fn apply(app: &mut PhotocraftApp, acts: Vec<Action>) {
    for a in acts {
        match a {
            Action::Select(name) => run_or_status(app, "tools.setBrush", json!({ "preset": name })),
            Action::Choose(name) => {
                // The double-click's first click already picked it.
                if app.session.tools.current_preset.as_deref() != Some(name.as_str()) {
                    run_or_status(app, "tools.setBrush", json!({ "preset": name }));
                }
            }
            Action::ToggleGroup(_) | Action::BeginRename(_) | Action::Click { .. } | Action::Deselect => {}
            Action::MoveItems { items, into, before, after, create } => {
                let mut p = json!({ "items": items.iter().map(BrushItem::param).collect::<Vec<_>>(), "into": into });
                if let Some(b) = before {
                    p["before"] = b.sibling();
                }
                if let Some(a) = after {
                    p["after"] = a.sibling();
                }
                if create {
                    p["create"] = json!(true);
                }
                run_or_status(app, "brush.presets.moveItems", p);
            }
            Action::Rename(r) => {
                let text = r.text.trim().to_string();
                let old = r.folder.last().unwrap_or(&r.name);
                if !text.is_empty() && text != *old {
                    if r.group {
                        let mut p = json!({ "group": r.name, "newName": text });
                        if !r.folder.is_empty() {
                            p["folder"] = json!(r.folder);
                        }
                        run_or_status(app, "brush.presets.renameGroup", p);
                    } else {
                        run_or_status(app, "brush.presets.rename", json!({ "name": r.name, "newName": text }));
                    }
                }
            }
            Action::Delete(name) => run_or_status(app, "brush.presets.delete", json!({ "name": name })),
            Action::DeleteGroup { group, folder } => {
                let mut p = json!({ "group": group });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                run_or_status(app, "brush.presets.deleteGroup", p);
            }
            Action::DeleteItems(items) => {
                run_or_status(app, "brush.presets.deleteItems", json!({ "items": items.iter().map(BrushItem::param).collect::<Vec<_>>() }));
            }
        }
    }
}

/// `a` blended toward `b` by `f` (0 = `a`), opaque.
fn mix(a: Color32, b: Color32, f: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// The panel list's colours, as in Photoshop: a darker well (`tab_strip` in the Pro themes)
/// behind panel-coloured tiles, hairlines under the group rows, a muted blue outline round the
/// selected tiles.
struct ListColors {
    well: Color32,
    tile: Color32,
    line: Color32,
    outline: Color32,
}

fn list_colors(t: &Tokens) -> ListColors {
    let well = if t.pro { t.tab_strip } else { t.field };
    ListColors { well, tile: t.card, line: mix(well, t.card, 0.4), outline: mix(t.card, t.accent, 0.6) }
}

/// The current selection as sets, for quick lookups while drawing (preset names lowercased,
/// as preset names match without case).
#[derive(Default)]
struct Sel {
    presets: HashSet<String>,
    folders: HashSet<Vec<String>>,
}

impl Sel {
    fn new(items: &[BrushItem]) -> Self {
        let mut s = Sel::default();
        for it in items {
            match it {
                BrushItem::Preset(n) => drop(s.presets.insert(n.to_lowercase())),
                BrushItem::Folder(f) => drop(s.folders.insert(f.clone())),
            }
        }
        s
    }

    fn has(&self, item: &BrushItem) -> bool {
        match item {
            BrushItem::Preset(n) => self.presets.contains(&n.to_lowercase()),
            BrushItem::Folder(f) => self.folders.contains(f),
        }
    }
}

/// How the list is drawn.
struct Draw<'a> {
    collapsed: &'a [String],
    filtering: bool,
    layout: ListLayout,
    /// The parts — name, stroke, tip — from the panel menu (the picker's gear).
    show: (bool, bool, bool),
    /// The picker cards' size scale (its footer slider, 1 = standard): multiplies [`CARD_MAX_W`]
    /// and [`CARD_MAX_H`], or [`CARD_TIP_ONLY_W`]/[`CARD_TIP_ONLY_H`] for tip-only cards and
    /// [`CARD_TIP_NAME_W`]/[`CARD_TIP_NAME_H`] for tip-and-name ones; the text holds its 1.0
    /// size ([`CARD_NAME_FONT`], [`CARD_NUM_FONT`]); at or below [`CARD_COMPACT_SCALE`] the tips
    /// drop their size numbers.
    scale: f32,
    /// The panel tiles' width (its footer slider).
    tile: f32,
    sel: Sel,
    /// The selection: what a drag of a selected item carries.
    selected: &'a [BrushItem],
    /// The items in the order drawn, for Shift ranges.
    visible: RefCell<Vec<BrushItem>>,
}

impl Draw<'_> {
    /// What dragging `item` moves: the whole selection when it's part of it.
    fn drag_items(&self, item: &BrushItem) -> Vec<BrushItem> {
        if self.selected.len() > 1 && self.sel.has(item) { self.selected.to_vec() } else { vec![item.clone()] }
    }
}

/// Is the pointer on the second half of `r` (lower in a column, right in a row)?
fn second_half(ui: &egui::Ui, r: egui::Rect, grid: bool) -> bool {
    ui.ctx().pointer_interact_pos().is_some_and(|p| if grid { p.x > r.center().x } else { p.y > r.center().y })
}

/// Selection clicks, drag, drop and context menu of one preset tile or card, which sits directly
/// in folder `parent`. `grid`: its neighbours are beside it (drops split left/right).
#[allow(clippy::too_many_arguments)]
fn preset_interactions(ui: &egui::Ui, d: &Draw, resp: &egui::Response, r: egui::Rect, p: &BrushPreset, parent: &[String], grid: bool, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let item = BrushItem::Preset(p.name.clone());
    if resp.drag_started() {
        resp.dnd_set_drag_payload(BrushDrag(d.drag_items(&item)));
    }
    let spot = if second_half(ui, r, grid) { Spot::After } else { Spot::Before };
    if let Some(dragged) = resp.dnd_hover_payload::<BrushDrag>()
        && drop_action(&dragged.0, &item, parent, spot).is_some()
    {
        widgets::drop_line(ui, r, spot == Spot::After, grid, &t);
    }
    if let Some(dragged) = resp.dnd_release_payload::<BrushDrag>() {
        acts.extend(drop_action(&dragged.0, &item, parent, spot));
    }
    let mods = ui.input(|i| i.modifiers);
    if resp.double_clicked() {
        if d.layout.cards {
            acts.push(Action::Choose(p.name.clone()));
        } else {
            // Photoshop's panel renames a brush on a double-click.
            acts.push(Action::BeginRename(Renaming { group: false, name: p.name.clone(), folder: Vec::new(), text: String::new() }));
        }
    } else if resp.clicked() {
        acts.push(Action::Click { item: item.clone(), shift: mods.shift, toggle: mods.command });
        if !mods.shift && !mods.command {
            acts.push(Action::Select(p.name.clone()));
        }
    }
    let several = d.selected.len() > 1 && d.sel.has(&item);
    resp.context_menu(|ui| {
        if ui.button(tl!("Rename Brush…")).clicked() {
            acts.push(Action::BeginRename(Renaming { group: false, name: p.name.clone(), folder: Vec::new(), text: String::new() }));
            ui.close();
        }
        if several {
            if ui.button(tl!("Delete Brushes")).clicked() {
                acts.push(Action::DeleteItems(d.selected.to_vec()));
                ui.close();
            }
        } else if ui.button(tl!("Delete Brush")).clicked() {
            acts.push(Action::Delete(p.name.clone()));
            ui.close();
        }
    });
}

/// The tile slider's range and default, in points (the width a tile asks for).
pub const TILE_RANGE: std::ops::RangeInclusive<f32> = 40.0..=320.0;
pub const TILE_DEFAULT: f32 = 84.0;
/// The gap between tiles, across and down: the well shows through as a grid line.
const TILE_GAP: f32 = 2.0;
/// Room kept right of the tiles (the scroll bar's).
const TILE_RIGHT: f32 = 2.0;
/// Padding inside a tile, and the name row under its stroke.
const TILE_PAD: f32 = 3.0;
const TILE_NAME_H: f32 = 14.0;
const TILE_NAME_FONT: f32 = 11.0;

/// The tiles' width and how many share a row in `room` points: as many `want`-wide tiles as fit,
/// stretched to fill the row (Photoshop's grid). At least one; whole points.
pub fn tile_columns(room: f32, want: f32) -> (f32, usize) {
    let room = if room.is_finite() { room.max(1.0) } else { 1.0 };
    let want = if want.is_finite() { want.clamp(*TILE_RANGE.start(), *TILE_RANGE.end()) } else { TILE_DEFAULT };
    let cols = (((room + TILE_GAP) / (want + TILE_GAP)).floor() as usize).clamp(1, 256);
    let w = ((room - TILE_GAP * (cols - 1) as f32) / cols as f32).floor().max(1.0);
    (w, cols)
}

/// The panel menu's parts with at least one on (none reads as stroke and name).
fn tile_parts(show: (bool, bool, bool)) -> (bool, bool, bool) {
    if show == (false, false, false) { (true, true, false) } else { show }
}

/// The height of a `w`-wide tile asked to be `want` wide: a stroke row (its height grows with
/// `want`) with the name under it; with the stroke off, a tip and the name in one row, the name
/// alone, or a square tip.
pub fn tile_height(show: (bool, bool, bool), want: f32, w: f32) -> f32 {
    let want = if want.is_finite() { want.clamp(*TILE_RANGE.start(), *TILE_RANGE.end()) } else { TILE_DEFAULT };
    let (name, stroke, tip) = tile_parts(show);
    let stroke_h = (want * 0.15).round().clamp(10.0, 64.0);
    let h = if stroke {
        let h = TILE_PAD * 2.0 + stroke_h + if name { TILE_NAME_H } else { 0.0 };
        if tip { h.max(36.0) } else { h }
    } else if tip && !name {
        w
    } else if tip {
        (want * 0.3).round().clamp(24.0, 96.0)
    } else {
        22.0
    };
    if h.is_finite() { h.max(1.0) } else { 22.0 }
}

/// One preset's tile in the panel: `w` × `h` points.
#[allow(clippy::too_many_arguments)]
fn tile(ui: &mut egui::Ui, p: &BrushPreset, d: &Draw, parent: &[String], w: f32, h: f32, grid: bool, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
    let item = BrushItem::Preset(p.name.clone());
    let selected = d.sel.has(&item);
    d.visible.borrow_mut().push(item);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &p.name));
    if ui.is_rect_visible(r) {
        paint_tile(ui, p, r, selected, resp.hovered(), d.show, &t);
    }
    let resp = resp.on_hover_text(&p.name);
    preset_interactions(ui, d, &resp, r, p, parent, grid, acts);
}

fn paint_tile(ui: &egui::Ui, p: &BrushPreset, r: egui::Rect, selected: bool, hovered: bool, show: (bool, bool, bool), t: &Tokens) {
    let c = list_colors(t);
    let painter = ui.painter();
    painter.rect_filled(r, 0.0, if hovered { mix(c.tile, t.hover, 0.5) } else { c.tile });
    if selected {
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, c.outline), egui::StrokeKind::Inside);
    }
    let (name, stroke, tip) = tile_parts(show);
    let inner = r.shrink(TILE_PAD);
    let pb = &p.brush;
    let mut x = inner.left();
    if tip {
        let tex = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{}", p.name), &pb.tip, (pb.hardness, pb.angle, pb.roundness), 36, t.text);
        if !stroke && !name {
            // A grid of tips with their sizes under them.
            let number = painter.layout_no_wrap(format!("{}", pb.size.round() as i64), egui::FontId::proportional(CARD_NUM_FONT), t.text_faint);
            let tr = card_tip_rect(inner, Some(number.size().y), pb.size, inner.width() / CARD_TIP_BASE);
            painter.image(tex.id(), tr, full_uv(), Color32::WHITE);
            let at = pos2(inner.center().x - number.size().x / 2.0, inner.bottom() - 1.0 - number.size().y);
            painter.galley(at, number, t.text_faint);
            return;
        }
        let cell = egui::Rect::from_min_size(inner.left_top(), egui::Vec2::splat(inner.height()));
        let tr = egui::Rect::from_center_size(cell.center(), cell.size() * crate::brush_sections::thumb_scale(pb.size));
        painter.image(tex.id(), tr, full_uv(), Color32::WHITE);
        x = cell.right() + TILE_PAD * 2.0;
    }
    let name_h = if name && stroke { TILE_NAME_H } else { 0.0 };
    if stroke {
        let sr = egui::Rect::from_min_max(pos2(x, inner.top()), pos2(inner.right(), inner.bottom() - name_h));
        // Rendered at the screen's pixel size (crisp at any scale); the width in steps of 4 pt so
        // resizing the panel re-renders less often.
        let ppp = ui.ctx().pixels_per_point();
        let sw = ((sr.width() / 4.0).floor() * 4.0).max(4.0);
        let (pw, ph) = ((sw * ppp).round().clamp(1.0, 4096.0) as u32, (sr.height() * ppp).round().clamp(1.0, 1024.0) as u32);
        let tex = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-tile:{}", p.name), pb, pw, ph, t.text);
        painter.image(tex.id(), egui::Rect::from_min_size(sr.left_top(), vec2(sw, sr.height())), full_uv(), Color32::WHITE);
    }
    if name {
        let cy = if stroke { inner.bottom() - TILE_NAME_H / 2.0 } else { inner.center().y };
        crate::layer_row_ui::label(painter, x + 2.0, cy, inner.right(), &p.name, egui::FontId::proportional(TILE_NAME_FONT), t.text);
    }
}

/// A run of presets in folder `parent` as rows of tiles, `indent` from the list's left edge.
fn tiles(ui: &mut egui::Ui, d: &Draw, run: &[&BrushPreset], parent: &[String], indent: f32, acts: &mut Vec<Action>) {
    let room = ui.available_width() - indent - TILE_RIGHT;
    let (w, cols) = tile_columns(room, d.tile);
    let h = tile_height(d.show, d.tile, w);
    ui.add_space(TILE_GAP);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(TILE_GAP, TILE_GAP);
        for row in run.chunks(cols) {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                for p in row {
                    tile(ui, p, d, parent, w, h, cols > 1, acts);
                }
            });
        }
    });
    ui.add_space(TILE_GAP);
}

/// The gap between grid cells, across and down.
const GRID_GAP: f32 = 3.0;
/// The picker's cards pack tighter than the grid: their horizontal gap (the vertical one stays
/// [`GRID_GAP`]).
const CARD_GAP: f32 = 1.0;

/// The grid's columns in a list `width` points wide: how many `cell`-wide cells separated by
/// `gap` fit past the `indent`, and the left margin that centres them in the room past it. At
/// least one column.
pub(crate) fn grid_columns(width: f32, cell: f32, indent: f32, gap: f32) -> (f32, usize) {
    let room = if width.is_finite() { (width - indent).max(0.0) } else { 0.0 };
    let cols = if cell.is_finite() && cell > 0.0 { ((room + gap) / (cell + gap)).floor() } else { 1.0 };
    // `as` saturates (and NaN becomes 0), so this is a whole number from 1 to a sane bound.
    let cols = (cols as usize).clamp(1, 256);
    let used = cols as f32 * cell + (cols - 1) as f32 * gap;
    // Whole points keep the tiles' edges crisp.
    let left = indent + ((room - used) / 2.0).max(0.0).floor();
    (if left.is_finite() { left } else { 0.0 }, cols)
}

/// Padding, tip cell width and name row height, at scale 1; the footer slider multiplies them
/// (heights down to [`CARD_MIN_H_SCALE`]).
const CARD_PAD: f32 = 4.0;
const CARD_TIP_W: f32 = 46.0;
/// The tip thumbnail's base size at scale 1, before the footer slider and the per-preset
/// [`crate::brush_sections::thumb_scale`]: the tips read small at 28 px, so the base carries a
/// 60 % bump.
const CARD_TIP_BASE: f32 = 28.0 * 1.6;
const CARD_NAME_H: f32 = 20.0;
/// A full card's height at scale 1: the tip/stroke row takes what the name row leaves. The
/// footer slider multiplies it down to [`CARD_MIN_H_SCALE`].
const CARD_MAX_H: f32 = 70.0;
/// The widest a lone card grows at scale 1; past it the room is shared between columns.
const CARD_MAX_W: f32 = 300.0;
/// Tip alone: a grid of small [`CARD_TIP_ONLY_W`] × [`CARD_TIP_ONLY_H`] cells (scale 1).
const CARD_TIP_ONLY_W: f32 = 60.0;
const CARD_TIP_ONLY_H: f32 = 70.0;
/// Tip + name, no stroke: a grid of [`CARD_TIP_NAME_W`] × [`CARD_TIP_NAME_H`] cells (scale 1).
const CARD_TIP_NAME_W: f32 = 150.0;
const CARD_TIP_NAME_H: f32 = 70.0;
/// The footer scale's floor for heights and padding (the widths keep shrinking).
const CARD_MIN_H_SCALE: f32 = 0.60;
/// At or below this scale the tips drop their size numbers and fill their cell whole.
const CARD_COMPACT_SCALE: f32 = 0.70;
/// stay readable from the smallest card to the largest.
const CARD_NAME_FONT: f32 = 12.0;
const CARD_NUM_FONT: f32 = 11.0;
/// The cards' own fill: [`Tokens::card`] (what the picker's popup is filled with) this much
/// darker, so the cards read as tiles against it.
const CARD_BG_SHADE: f32 = 0.90;

/// The picker's cards in a list `width` points wide, `indent` past its left edge: how wide each
/// card is and how many sit in a row. A lone card fills the room up to [`CARD_MAX_W`] × `scale`
/// (wider lists leave the rest empty); from two columns on, the cards share the room, filling it
/// fully. `scale` is the picker's footer slider ([`crate::brush_picker::body`]), already floored
/// so the widths never reach zero.
fn card_columns(width: f32, indent: f32, scale: f32) -> (f32, usize) {
    let max_w = CARD_MAX_W * scale;
    let room = if width.is_finite() { (width - indent).max(0.0) } else { CARD_MAX_W };
    let cols = ((room / max_w).floor() as usize).clamp(1, 256);
    let share = (room - CARD_GAP * (cols - 1) as f32) / cols as f32;
    let card_w = if cols == 1 { room.min(max_w) } else { share };
    (if card_w.is_finite() { card_w.max(1.0) } else { CARD_MAX_W }, cols)
}

/// Is the card showing the tip alone (no name, no stroke)? Then it's a small tip cell in a grid.
fn tip_only(show: (bool, bool, bool)) -> bool {
    show == (false, false, true)
}

/// Is the card showing the tip and the name with the stroke off? Then it's a tip-and-name cell.
fn tip_and_name(show: (bool, bool, bool)) -> bool {
    show == (true, false, true)
}

/// `c` darker by `f` in gamma space, kept opaque — [`Color32::gamma_multiply`] fades the alpha
/// along with the channels, so it can't darken a fill over its own color.
fn darker(c: Color32, f: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied((r as f32 * f).round() as u8, (g as f32 * f).round() as u8, (b as f32 * f).round() as u8, a)
}

/// Where a card's tip thumbnail goes in its `cell`. `number_h` is the height of the size number
/// under it, `None` on compact cards (no number: the tip takes the cell whole). Otherwise the
/// number keeps a band at the bottom of the cell and the tip is centred in what's left above it,
/// at [`CARD_TIP_BASE`] × `scale` or that room's side, whichever is smaller, times the preset's
/// [`crate::brush_sections::thumb_scale`]: big brushes still read bigger than small ones, and
/// none of them reaches down over the number.
fn card_tip_rect(cell: egui::Rect, number_h: Option<f32>, size: f32, scale: f32) -> egui::Rect {
    let Some(number_h) = number_h else {
        return egui::Rect::from_center_size(cell.center(), egui::Vec2::splat((cell.width().min(cell.height()) - 2.0).max(1.0)));
    };
    // The number sits 1 pt off the cell's bottom, with a 1 pt gap above it.
    let room = egui::Rect::from_min_max(cell.min, pos2(cell.right(), (cell.bottom() - number_h - 2.0).max(cell.top())));
    let side = (CARD_TIP_BASE * scale).min(room.width().min(room.height())).max(0.0) * crate::brush_sections::thumb_scale(size);
    egui::Rect::from_center_size(room.center(), egui::Vec2::splat(if side.is_finite() { side.max(1.0) } else { 1.0 }))
}

/// One preset in the Brush Preset picker: a 2 × 2 grid whose bottom row (the name) spans both
/// columns — tip at the top left, stroke preview at the top right. A part that's off loses its
/// cell (the other takes the room): with only the tip and the name on, the name moves into the
/// stroke's cell and the bottom row is gone. At least one part is always on. `d.show` is the
/// three gear boxes (name, stroke, tip); `width` is the row slot the card fills (a
/// [`card_columns`] column or a fixed cell, [`CARD_TIP_ONLY_W`]/[`CARD_TIP_NAME_W`]), and `scale`
/// the footer slider: heights and padding floor at [`CARD_MIN_H_SCALE`], the text holds its 1.0
/// size ([`CARD_NAME_FONT`], [`CARD_NUM_FONT`]), and at or below [`CARD_COMPACT_SCALE`] the tips
/// drop their size numbers and take their cell whole.
fn preset_card(ui: &mut egui::Ui, p: &BrushPreset, d: &Draw, parent: &[String], width: f32, scale: f32, acts: &mut Vec<Action>) {
    let show = d.show;
    let (show_name, show_stroke, show_tip) = show;
    let tip_only = tip_only(show);
    let t = Tokens::get(ui.ctx());
    let name_in_top = tip_and_name(show);
    let item = BrushItem::Preset(p.name.clone());
    let current = d.sel.has(&item);
    d.visible.borrow_mut().push(item);
    // Widths (which arrive scaled) keep narrowing with the slider and heights stop at the floor;
    // the text holds its 1.0 size — the fonts don't take the scale.
    let scale_h = scale.max(CARD_MIN_H_SCALE);
    let pad_y = CARD_PAD * scale_h;
    let name_h = if show_name && !name_in_top { CARD_NAME_H * scale_h } else { 0.0 };
    // The tip/stroke row: the tip-only or tip-and-name card's own height less the padding, else
    // what [`CARD_MAX_H`] leaves past the padding and the name row. A card with no top part at
    // all (the name alone) has no such row.
    let top_h = if tip_only {
        (CARD_TIP_ONLY_H - CARD_PAD * 2.0) * scale_h
    } else if name_in_top {
        (CARD_TIP_NAME_H - CARD_PAD * 2.0) * scale_h
    } else if show_tip || show_stroke {
        (CARD_MAX_H - CARD_PAD * 2.0 - CARD_NAME_H) * scale_h
    } else {
        0.0
    };
    let (full, resp) = ui.allocate_exact_size(vec2(width, pad_y * 2.0 + top_h + name_h), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &p.name));
    // The card is inset a little in its column; the whole column stays the drop target.
    let card = full.shrink2(vec2((4.0 * scale).min(full.width() * 0.5), 0.0));
    if ui.is_rect_visible(full) {
        paint_card(ui, p, card, current, resp.hovered(), show, scale, (top_h, name_h), &t);
    }
    let resp = resp.on_hover_text(&p.name);
    // A tip-only card's grid drops between columns: the rest drop between rows (there is a row
    // under the tip there).
    preset_interactions(ui, d, &resp, card, p, parent, tip_only, acts);
}

/// Paint a picker card (see [`preset_card`]); `rows` is the tip/stroke row's and the name row's
/// height.
#[allow(clippy::too_many_arguments)]
fn paint_card(ui: &egui::Ui, p: &BrushPreset, card: egui::Rect, current: bool, hovered: bool, show: (bool, bool, bool), scale: f32, rows: (f32, f32), t: &Tokens) {
    let (show_name, show_stroke, show_tip) = show;
    let tip_only = tip_only(show);
    let name_in_top = tip_and_name(show);
    let compact = scale <= CARD_COMPACT_SCALE;
    let scale_h = scale.max(CARD_MIN_H_SCALE);
    let (pad, pad_y) = (CARD_PAD * scale, CARD_PAD * scale_h);
    let (top_h, name_h) = rows;
    // Every card carries its own background, a touch darker than the popup behind it, so the
    // cards read as tiles; the current and hover fills paint over it.
    ui.painter().rect_filled(card, 3.0, darker(t.card, CARD_BG_SHADE));
    if current {
        ui.painter().rect_filled(card, 3.0, t.accent_soft);
        ui.painter().rect_stroke(card, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_filled(card, 3.0, t.hover);
    }
    let pb = &p.brush;
    let inner = card.shrink2(vec2(pad, pad_y));
    let top = egui::Rect::from_min_size(inner.left_top(), vec2(inner.width(), top_h));
    let mut x = top.left();
    if show_tip {
        // The tip thumbnail with the size under it (like the list row). With the tip alone the
        // cell takes the whole card, so the thumbnail is centred in it (the tip-only grid).
        // Compact cards drop the number — it wouldn't fit — and the tip takes the cell whole.
        let tip_w = if tip_only { top.width() } else { (CARD_TIP_W * scale).min(top.width() * 0.5) };
        let cell = egui::Rect::from_min_size(top.left_top(), vec2(tip_w, top_h));
        let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{}", p.name), &pb.tip, (pb.hardness, pb.angle, pb.roundness), 36, t.text);
        let number =
            (!compact).then(|| ui.painter().layout_no_wrap(format!("{}", pb.size.round() as i64), egui::FontId::proportional(CARD_NUM_FONT), t.text_faint));
        let tr = card_tip_rect(cell, number.as_ref().map(|g| g.size().y), pb.size, scale);
        ui.painter().image(tip.id(), tr, full_uv(), Color32::WHITE);
        if let Some(g) = number {
            let at = pos2(cell.center().x - g.size().x / 2.0, cell.bottom() - 1.0 - g.size().y);
            ui.painter().galley(at, g, t.text_faint);
        }
        x = cell.right();
    }
    if show_stroke || name_in_top {
        // The stroke preview, or the name where it goes when the stroke is hidden.
        let cell = egui::Rect::from_min_max(pos2(x, top.top()), top.right_bottom());
        if show_stroke {
            let cw = if cell.width().is_finite() { cell.width().max(1.0) } else { 1.0 };
            let w = ((cw - 16.0 * scale).clamp(24.0 * scale, 260.0 * scale)).min(cw).max(1.0).round();
            let h = (36.0 * scale_h).round().max(1.0) as u32;
            let stroke = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-stroke-{w}:{}", p.name), pb, w as u32, h, t.text);
            let sr = egui::Rect::from_min_size(pos2(cell.left() + 8.0 * scale, cell.center().y - 18.0 * scale_h), vec2(w, 36.0 * scale_h));
            ui.painter().image(stroke.id(), sr, full_uv(), Color32::WHITE);
        } else {
            crate::layer_row_ui::label(
                ui.painter(),
                cell.left() + 8.0 * scale,
                cell.center().y,
                cell.right() - 6.0 * scale,
                &p.name,
                egui::FontId::proportional(CARD_NAME_FONT),
                t.text_dim,
            );
        }
    }
    if name_h > 0.0 && show_name {
        // The merged bottom row: the name across the whole card.
        let cell = egui::Rect::from_min_size(pos2(inner.left(), top.bottom()), vec2(inner.width(), name_h));
        crate::layer_row_ui::label(
            ui.painter(),
            cell.left() + 6.0 * scale,
            cell.center().y,
            cell.right() - 6.0 * scale,
            &p.name,
            egui::FontId::proportional(CARD_NAME_FONT),
            t.text_dim,
        );
    }
}

/// Indent of one nested folder level.
const FOLDER_INDENT: f32 = 16.0;
/// A group or folder row's height (Photoshop's).
const FOLDER_ROW_H: f32 = 26.0;

/// A group's or nested folder's row, as in Photoshop: chevron, folder, name. A click selects it
/// (Shift and Ctrl extend the selection), the chevron opens and closes it, a double-click renames
/// it. Items dropped on its upper edge land before it, on a closed folder's lower edge after it,
/// and on the rest of the row inside it.
fn folder_row(ui: &mut egui::Ui, d: &Draw, f: &FolderNode, open: bool, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let c = list_colors(&t);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FOLDER_ROW_H), Sense::click_and_drag());
    let item = BrushItem::Folder(f.path.clone());
    let selected = d.sel.has(&item);
    d.visible.borrow_mut().push(item.clone());
    let shown = f.path.last().map_or("", String::as_str);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, selected, shown));
    let parent = f.path.split_last().map(|(_, p)| p).unwrap_or_default();
    let x = r.left() + d.layout.indent + FOLDER_INDENT * parent.len().min(MAX_VIEW_DEPTH) as f32;
    // The chevron is a button of its own, over the row.
    let chevron = egui::Rect::from_min_size(pos2(x - 2.0, r.top()), vec2(18.0, r.height()));
    let chev = ui.interact(chevron, resp.id.with("chevron"), Sense::click());
    chev.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} {shown}", if open { "Collapse" } else { "Expand" })));
    if ui.is_rect_visible(r) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(r, 0.0, t.row_selected);
        } else if resp.hovered() || chev.hovered() {
            painter.rect_filled(r, 0.0, mix(c.well, t.hover, 0.5));
        }
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, c.line));
        icons::paint(ui, egui::Rect::from_center_size(chevron.center(), vec2(12.0, 12.0)), if open { "chevron-down" } else { "chevron-right" }, 11.0, t.text_faint);
        crate::layer_row_ui::paint_folder(painter, egui::Rect::from_min_size(pos2((x + 18.0).round(), (r.center().y - 6.0).round()), vec2(16.0, 12.0)), t.icon);
        let right = if t.pro { r.right() - 6.0 } else { r.right() - 34.0 };
        crate::layer_row_ui::label(painter, x + 42.0, r.center().y, right, shown, egui::FontId::proportional(12.0), t.text);
        if !t.pro {
            painter.text(pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, f.count.to_string(), egui::FontId::proportional(11.0), t.text_faint);
        }
    }
    if chev.clicked() {
        acts.push(Action::ToggleGroup(folder_view_key(&f.path)));
    }
    if resp.drag_started() {
        resp.dnd_set_drag_payload(BrushDrag(d.drag_items(&item)));
    }
    let spot = ui.ctx().pointer_interact_pos().map_or(Spot::Into, |p| {
        if p.y < r.top() + r.height() * 0.3 {
            Spot::Before
        } else if !open && p.y > r.bottom() - r.height() * 0.3 {
            Spot::After
        } else {
            Spot::Into
        }
    });
    if let Some(dragged) = resp.dnd_hover_payload::<BrushDrag>()
        && drop_action(&dragged.0, &item, parent, spot).is_some()
    {
        match spot {
            Spot::Into => {
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            }
            s => widgets::drop_line(ui, r, s == Spot::After, false, &t),
        }
    }
    if let Some(dragged) = resp.dnd_release_payload::<BrushDrag>() {
        acts.extend(drop_action(&dragged.0, &item, parent, spot));
    }
    let mods = ui.input(|i| i.modifiers);
    if resp.double_clicked() {
        acts.push(Action::BeginRename(renaming_folder(&f.path)));
    } else if resp.clicked() {
        acts.push(Action::Click { item: item.clone(), shift: mods.shift, toggle: mods.command });
    }
    let several = d.selected.len() > 1 && selected;
    resp.context_menu(|ui| {
        if ui.button(tl!("Rename Group…")).clicked() {
            acts.push(Action::BeginRename(renaming_folder(&f.path)));
            ui.close();
        }
        if several {
            if ui.button(tl!("Delete Brushes")).clicked() {
                acts.push(Action::DeleteItems(d.selected.to_vec()));
                ui.close();
            }
        } else if ui.button(tl!("Delete Group")).clicked() {
            let (group, folder) = f.path.split_first().map(|(g, rest)| (g.clone(), rest.to_vec())).unwrap_or_default();
            acts.push(Action::DeleteGroup { group, folder });
            ui.close();
        }
    });
}

/// The rename of the group or folder at `path`.
fn renaming_folder(path: &[String]) -> Renaming {
    let (name, folder) = path.split_first().map(|(g, rest)| (g.clone(), rest.to_vec())).unwrap_or_default();
    Renaming { group: true, name, folder, text: String::new() }
}

/// Draw `nodes` (a level of the tree, directly in folder `parent`): runs of presets as tiles or
/// cards, folders as rows with their content below when open. The depth is bounded by
/// [`folder_tree`] (paths are cut at [`MAX_VIEW_PATH`]).
fn draw_nodes(ui: &mut egui::Ui, d: &Draw, nodes: &[Node], parent: &[String], acts: &mut Vec<Action>) {
    let depth = parent.len();
    let mut run: Vec<&BrushPreset> = Vec::new();
    let flush = |ui: &mut egui::Ui, run: &mut Vec<&BrushPreset>, acts: &mut Vec<Action>| {
        if run.is_empty() {
            return;
        }
        if d.layout.cards {
            // The picker's cards: [`card_columns`] cards sharing each row, a centred grid of
            // small [`CARD_TIP_ONLY_W`] cells, or card-wide [`CARD_TIP_NAME_W`] cells snapped
            // to the indent like the full-card rows. The scale floors at the slider's own
            // bottom end. A group's own presets aren't indented; nested folders' are.
            let indent = FOLDER_INDENT * depth.saturating_sub(1) as f32;
            let scale = d.scale.max(*crate::brush_picker::SCALE_RANGE.start());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(CARD_GAP, GRID_GAP);
                let fixed = if tip_only(d.show) {
                    Some(CARD_TIP_ONLY_W * scale)
                } else if tip_and_name(d.show) {
                    Some(CARD_TIP_NAME_W * scale)
                } else {
                    None
                };
                if let Some(cell) = fixed {
                    // Tip-only cells centre their whole columns like a grid; the card-sized
                    // tip-and-name cells start at the indent.
                    let (mut left, cols) = grid_columns(ui.available_width(), cell, indent, CARD_GAP);
                    if tip_and_name(d.show) {
                        left = indent;
                    }
                    for row in run.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.add_space(left);
                            for p in row {
                                preset_card(ui, p, d, parent, cell, scale, acts);
                            }
                        });
                    }
                } else {
                    let (card_w, cols) = card_columns(ui.available_width(), indent, scale);
                    for row in run.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            for p in row {
                                preset_card(ui, p, d, parent, card_w, scale, acts);
                            }
                        });
                    }
                }
            });
        } else {
            tiles(ui, d, run, parent, d.layout.indent + FOLDER_INDENT * depth as f32, acts);
        }
        run.clear();
    };
    for n in nodes {
        match n {
            Node::Preset(p) => run.push(p),
            Node::Folder(f) => {
                flush(ui, &mut run, acts);
                let open = d.filtering || !d.collapsed.contains(&folder_view_key(&f.path));
                folder_row(ui, d, f, open, acts);
                if open {
                    draw_nodes(ui, d, &f.children, &f.path, acts);
                }
            }
        }
    }
    flush(ui, &mut run, acts);
}

/// The rename bar shown while a preset or group is being renamed. Enter or OK renames, Escape or
/// Cancel stops.
fn rename_bar(ui: &mut egui::Ui, st: &mut BrushesPanelState, acts: &mut Vec<Action>) {
    let Some(r) = st.renaming.as_mut() else { return };
    let t = Tokens::get(ui.ctx());
    if r.text.is_empty() {
        r.text = r.folder.last().unwrap_or(&r.name).clone();
    }
    let mut done = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(if r.group { tl!("Group name") } else { tl!("Brush name") }).color(t.text_dim));
        let resp = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width((ui.available_width() - 140.0).max(60.0)).id_salt("brush-rename"));
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = widgets::dialog_buttons(
            ui,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("OK"), 52.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 60.0),
            ],
        );
        if clicked == Some(widgets::ButtonRole::Default) || enter {
            acts.push(Action::Rename(r.clone()));
            done = true;
        }
        if clicked == Some(widgets::ButtonRole::Cancel) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
    });
    if done {
        st.renaming = None;
    }
    ui.add_space(4.0);
}

/// Keep the selection in step with the library: drop items that are gone, and select the current
/// preset whenever it changes (picked here or anywhere else).
fn sync_selection(st: &mut BrushesPanelState, presets: &[BrushPreset], current: Option<&str>) {
    if st.followed.as_deref() != current {
        st.followed = current.map(str::to_string);
        if let Some(p) = current.and_then(|c| presets.iter().find(|p| p.name.eq_ignore_ascii_case(c))) {
            let it = BrushItem::Preset(p.name.clone());
            st.selected = vec![it.clone()];
            st.anchor = Some(it);
        }
    }
    if st.selected.is_empty() {
        return;
    }
    let names: HashSet<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    st.selected.retain(|it| match it {
        BrushItem::Preset(n) => names.contains(n.as_str()),
        BrushItem::Folder(f) => presets.iter().any(|p| under(p, f)),
    });
}

/// The presets in their tree, filtered by `st.filter`, as the panel's tiles or the picker's
/// cards (`layout.cards`), from the `st.show_*` parts, with the rename bar above them while a
/// rename is open. `current` is the current preset's name; the selection follows it when it
/// changes. Group toggles, selection clicks and renames update `st`; the returned actions are
/// commands for [`apply`].
pub fn preset_list(ui: &mut egui::Ui, presets: &[BrushPreset], current: Option<&str>, st: &mut BrushesPanelState, layout: ListLayout) -> Vec<Action> {
    let mut acts = Vec::new();
    sync_selection(st, presets, current);
    rename_bar(ui, st, &mut acts);
    let filter = st.filter.trim().to_lowercase();
    let items: Vec<&BrushPreset> = presets.iter().filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter)).collect();
    let tree = folder_tree(&items);
    let d = Draw {
        collapsed: &st.collapsed,
        filtering: !filter.is_empty(),
        layout,
        show: if layout.cards { (st.show_name, st.show_stroke, st.show_tip) } else { tile_parts((st.show_name, st.show_stroke, st.show_tip)) },
        scale: st.scale,
        tile: st.tile,
        sel: Sel::new(&st.selected),
        selected: &st.selected,
        visible: RefCell::new(Vec::new()),
    };
    let t = Tokens::get(ui.ctx());
    let well = (!layout.cards).then(|| list_colors(&t).well);
    egui::Frame::new().fill(well.unwrap_or(Color32::TRANSPARENT)).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(layout.id).max_height(layout.max_height).auto_shrink([false, well.is_none()]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = if layout.cards { 1.0 } else { 0.0 };
            draw_nodes(ui, &d, &tree, &[], &mut acts);
            if !layout.cards {
                // The rest of the list: a drop here goes to the end of the top level, a click
                // clears the selection.
                let room = (ui.clip_rect().bottom() - ui.cursor().top()).max(FOLDER_ROW_H);
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), room), Sense::click());
                if let Some(dragged) = resp.dnd_hover_payload::<BrushDrag>()
                    && !dragged.0.is_empty()
                {
                    widgets::drop_line(ui, egui::Rect::from_min_size(r.min, vec2(r.width(), 4.0)), false, false, &t);
                }
                if let Some(dragged) = resp.dnd_release_payload::<BrushDrag>()
                    && !dragged.0.is_empty()
                {
                    acts.push(Action::MoveItems { items: dragged.0.clone(), into: Vec::new(), before: None, after: None, create: false });
                }
                if resp.clicked() {
                    acts.push(Action::Deselect);
                }
            }
            if presets.is_empty() {
                ui.label(RichText::new(tl!("No brush presets")).color(t.text_faint));
            }
        });
    });
    // The dragged items follow the pointer.
    if let Some(dragged) = egui::DragAndDrop::payload::<BrushDrag>(ui.ctx())
        && let Some(p) = ui.ctx().pointer_interact_pos()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        // The first item's name, and how many more come with it.
        let name = match dragged.0.as_slice() {
            [] => String::new(),
            [one] => one.label().to_string(),
            [first, rest @ ..] => format!("{} +{}", first.label(), rest.len()),
        };
        egui::Area::new(egui::Id::new(("brush-drag-label", layout.id))).order(egui::Order::Tooltip).fixed_pos(p + vec2(12.0, 8.0)).interactable(false).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
            },
        );
    }
    let visible = d.visible.into_inner();
    view_actions(st, acts, &visible)
}

/// The Brushes panel's own items at the top of its panel menu (Photoshop's flyout): new brush and
/// group, rename and delete the selection, which parts the tiles show, and importing brushes.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.button(tl!("New Brush Preset…")).clicked() {
        crate::brush_preset_dialog::open(app, None);
        ui.close();
    }
    let group = new_group(&app.session.tools.presets, &app.ui.brushes_panel.selected);
    if ui.add_enabled(group.is_some(), egui::Button::new(tl!("New Group"))).clicked()
        && let Some((act, renaming)) = group
    {
        apply(app, vec![act]);
        app.ui.brushes_panel.renaming = Some(renaming);
        ui.close();
    }
    let one = match app.ui.brushes_panel.selected.as_slice() {
        [BrushItem::Preset(n)] => Some(Renaming { group: false, name: n.clone(), folder: Vec::new(), text: String::new() }),
        [BrushItem::Folder(f)] => Some(renaming_folder(f)),
        _ => None,
    };
    if ui.add_enabled(one.is_some(), egui::Button::new(tl!("Rename…"))).clicked() {
        app.ui.brushes_panel.renaming = one;
        ui.close();
    }
    let selected = app.ui.brushes_panel.selected.clone();
    if ui.add_enabled(!selected.is_empty(), egui::Button::new(tl!("Delete"))).clicked() {
        apply(app, vec![Action::DeleteItems(selected)]);
        ui.close();
    }
    ui.separator();
    // Photoshop's Brush Name / Brush Stroke / Brush Tip: at least one stays on.
    let st = &mut app.ui.brushes_panel;
    let (mut name, mut stroke, mut tip) = tile_parts((st.show_name, st.show_stroke, st.show_tip));
    let on = usize::from(name) + usize::from(stroke) + usize::from(tip);
    ui.add_enabled_ui(!(name && on == 1), |ui| ui.checkbox(&mut name, tl!("Brush Name")));
    ui.add_enabled_ui(!(stroke && on == 1), |ui| ui.checkbox(&mut stroke, tl!("Brush Stroke")));
    ui.add_enabled_ui(!(tip && on == 1), |ui| ui.checkbox(&mut tip, tl!("Brush Tip")));
    (st.show_name, st.show_stroke, st.show_tip) = (name, stroke, tip);
    ui.separator();
    if ui.button(tl!("Import Brushes…")).clicked() {
        let _ = app.open_dialog_file();
        ui.close();
    }
}

/// The Brushes panel.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // A dock panel: everything follows its width (the slider and search field) and the list takes
    // the height left above the footer.
    let width = if ui.available_width().is_finite() { ui.available_width().clamp(160.0, WIDTH) } else { WIDTH };
    let avail_h = ui.available_height();
    // Size of the current brush (Photoshop's Brushes panel slider).
    let before = app.session.tools.brush.clone();
    let mut b = before.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size")).color(t.text_dim));
        let mut lv = b.size.max(1.0).ln();
        ui.add_sized(vec2((width - 140.0).max(40.0), 18.0), |ui: &mut egui::Ui| {
            let r = widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None);
            if r.changed() {
                b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
            }
            r
        });
        let mut s = b.size;
        if widgets::value_field(ui, &mut s, 1.0..=MAX_BRUSH_SIZE, "px", 74.0).changed() {
            b.size = s.round().clamp(1.0, MAX_BRUSH_SIZE);
        }
    });
    commit_gesture(app, ui.ctx(), &before, &b);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        icons::paint(ui, egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), "search", 14.0, t.text_faint);
        ui.add_space(20.0);
        widgets::text_field(ui, ui.id().with("brush-search"), &mut app.ui.brushes_panel.filter, tl!("Search Brushes"), (ui.available_width() - 4.0).max(60.0), false);
    });
    ui.add_space(6.0);
    // The list fills the panel down to the footer (about 36 pt), so the footer stays in view in
    // a short group.
    let used = (avail_h - ui.available_height()).max(0.0);
    let list_h = if avail_h.is_finite() { (avail_h - used - 40.0).max(40.0) } else { PANEL_LIST.max_height };
    let layout = ListLayout { max_height: list_h, ..PANEL_LIST };
    let mut acts = preset_list(ui, &app.session.tools.presets, app.session.tools.current_preset.as_deref(), &mut app.ui.brushes_panel, layout);
    ui.add_space(4.0);
    // Photoshop's footer: the Brush Settings toggle and the thumbnail size slider on the left; new
    // group, new brush and delete on the right.
    ui.horizontal(|ui| {
        crate::brush_picker::settings_toggle(app, ui);
        let st = &mut app.ui.brushes_panel;
        let mut tile = if st.tile.is_finite() { st.tile.clamp(*TILE_RANGE.start(), *TILE_RANGE.end()) } else { TILE_DEFAULT };
        ui.add_sized(vec2((width * 0.3).clamp(60.0, 120.0), 18.0), |ui: &mut egui::Ui| widgets::slider(ui, &mut tile, TILE_RANGE, None))
            .on_hover_text(tl!("Thumbnail size"));
        st.tile = tile;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The selection, else the current preset.
            let current = app.session.tools.current_preset.clone().filter(|n| app.session.tools.presets.iter().any(|p| p.name.eq_ignore_ascii_case(n)));
            let doomed = match (app.ui.brushes_panel.selected.is_empty(), current) {
                (false, _) => app.ui.brushes_panel.selected.clone(),
                (true, Some(c)) => vec![BrushItem::Preset(c)],
                (true, None) => Vec::new(),
            };
            if ui.add_enabled_ui(!doomed.is_empty(), |ui| icons::button(ui, "trash", 24.0, false, "Delete brush")).inner.clicked() {
                acts.push(Action::DeleteItems(doomed));
            }
            let new = icons::button(ui, "square-plus", 24.0, false, "Create new brush from the current settings");
            if crate::brush_picker::named(new, "Create new brush from the current settings").clicked() {
                crate::brush_preset_dialog::open(app, None);
            }
            let group = new_group(&app.session.tools.presets, &app.ui.brushes_panel.selected);
            if crate::brush_picker::named(ui.add_enabled_ui(group.is_some(), |ui| icons::button(ui, "folder-plus", 24.0, false, "Create a new group")).inner, "Create a new group")
                .clicked()
                && let Some((act, renaming)) = group
            {
                acts.push(act);
                app.ui.brushes_panel.renaming = Some(renaming);
            }
            if icons::button(ui, "folder-open", 24.0, false, "Import Brushes… (.abr)").clicked() {
                let _ = app.open_dialog_file();
            }
        });
    });
    apply(app, acts);
    // Forget previews of presets that no longer exist.
    let names: HashSet<&str> = app.session.tools.presets.iter().map(|p| p.name.as_str()).collect();
    brush_preview::with_cache(ui.ctx(), |c| {
        c.retain(|slot| slot.split_once(':').is_none_or(|(_, n)| names.contains(n)));
    });
}

#[cfg(test)]
#[path = "brushes_tab_tests.rs"]
mod tests;
