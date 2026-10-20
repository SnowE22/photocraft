//! Saved Topaz Gigapixel AI settings: File › Automate › Topaz Gigapixel Presets runs one with a
//! click, through Gigapixel's command-line tool, without opening its window.
//!
//! A preset is a name plus the settings `file.automate.topazGigapixel` takes, in one of two
//! shapes:
//! - one upscale: `{"model"?, "scale" | "width" | "height", "denoise"?, …}` ([`super::cli_args`]);
//! - two (up to [`MAX_STEPS`]) saved one-upscale presets: `{"presets": [a, b], "combine"}`, where
//!   `"chain"` upscales the result of `a` again with `b` and `"layers"` upscales the image with
//!   each and keeps both results as layers (to compare or blend them).
//!
//! They persist with the other presets in the preferences document (`gigapixel_presets`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::presets::{always, bad, req_str};
use crate::{Result, Session};

pub const LIST_ID: &str = "file.automate.topazGigapixel.presets.list";
pub const SAVE_ID: &str = "file.automate.topazGigapixel.presets.save";
pub const DELETE_ID: &str = "file.automate.topazGigapixel.presets.delete";

const MAX_PRESETS: usize = 256;
/// The most presets one run combines.
pub const MAX_STEPS: usize = 4;
/// The settings of one upscale.
const UPSCALE_KEYS: [&str; 11] =
    ["model", "modelVersion", "scale", "width", "height", "denoise", "sharpen", "compression", "detail", "faceRecovery", "faceRecoveryVersion"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GigapixelPreset {
    pub name: String,
    /// `file.automate.topazGigapixel` params: one upscale, or `presets` + `combine`.
    pub settings: Value,
}

impl GigapixelPreset {
    /// The presets this one runs in turn, when it combines saved presets.
    pub fn combined(&self) -> Option<Vec<&str>> {
        self.settings.get("presets")?.as_array().map(|a| a.iter().filter_map(Value::as_str).collect())
    }
}

/// How the results of several presets end up in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// Each preset upscales the previous one's result; the last result is the new layer.
    Chain,
    /// Each preset upscales the image; every result becomes a layer.
    Layers,
}

impl Combine {
    pub fn parse(v: Option<&Value>, cmd: &str) -> Result<Combine> {
        match v.filter(|v| !v.is_null()).map(Value::as_str) {
            None | Some(Some("chain")) => Ok(Combine::Chain),
            Some(Some("layers")) => Ok(Combine::Layers),
            _ => Err(bad(cmd, "`combine` must be \"chain\" or \"layers\"")),
        }
    }
}

fn checked_name(name: &str) -> std::result::Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > crate::brush_preset_cmds::MAX_NAME {
        return Err("Preset names must contain 1–255 characters after trimming".into());
    }
    Ok(name.into())
}

/// The settings without null fields, checked: one upscale [`super::cli_args`] accepts, or 2 to
/// [`MAX_STEPS`] preset names and how to combine them.
fn checked_settings(v: &Value, cmd: &str) -> Result<Value> {
    let o = v.as_object().ok_or_else(|| bad(cmd, "`settings` must be an object"))?;
    let o: Map<String, Value> = o.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some(names) = o.get("presets") {
        if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "presets" | "combine")) {
            return Err(bad(cmd, format!("`{k}` doesn't go with `presets`: set it in the presets combined")));
        }
        let names = names.as_array().ok_or_else(|| bad(cmd, "`presets` must be a list of preset names"))?;
        if !(2..=MAX_STEPS).contains(&names.len()) {
            return Err(bad(cmd, format!("`presets` names 2 to {MAX_STEPS} presets")));
        }
        let names: Vec<String> = names
            .iter()
            .map(|n| n.as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "`presets` must be a list of preset names")))
            .collect::<Result<_>>()?;
        let combine = match Combine::parse(o.get("combine"), cmd)? {
            Combine::Chain => "chain",
            Combine::Layers => "layers",
        };
        return Ok(json!({"presets": names, "combine": combine}));
    }
    if let Some(k) = o.keys().find(|k| !UPSCALE_KEYS.contains(&k.as_str())) {
        return Err(bad(cmd, format!("unknown setting `{k}`")));
    }
    let settings = Value::Object(o);
    super::cli_args(&settings, std::path::Path::new("in.tif"), std::path::Path::new("out")).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(settings)
}

/// Read entries independently so a damaged preset cannot discard other presets or preferences.
pub(crate) fn load(value: Value) -> Vec<GigapixelPreset> {
    let Some(entries) = value.as_array() else { return Vec::new() };
    let mut presets: Vec<GigapixelPreset> = Vec::new();
    for entry in entries.iter().take(MAX_PRESETS) {
        let Ok(preset) = serde_json::from_value::<GigapixelPreset>(entry.clone()) else { continue };
        let Ok(name) = checked_name(&preset.name) else { continue };
        let Ok(settings) = checked_settings(&preset.settings, SAVE_ID) else { continue };
        if presets.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        presets.push(GigapixelPreset { name, settings });
    }
    presets
}

/// The saved preset called `name` (any case).
pub fn find<'a>(s: &'a Session, name: &str) -> Option<&'a GigapixelPreset> {
    s.presets.gigapixel_presets.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

/// What running `names` together means: each one's upscale settings, labelled by its name. A
/// preset that combines others can't be combined again.
pub fn steps_of(s: &Session, names: &[&str], cmd: &str) -> Result<Vec<(String, Value)>> {
    names
        .iter()
        .map(|n| {
            let p = find(s, n).ok_or_else(|| bad(cmd, format!("there is no Topaz Gigapixel preset {n:?}")))?;
            if p.combined().is_some() {
                return Err(bad(cmd, format!("“{}” combines presets itself; combine one-upscale presets", p.name)));
            }
            Ok((p.name.clone(), p.settings.clone()))
        })
        .collect()
}

/// `{"name", "settings", "replace"?}`: save a preset, overwriting the one of the same name; with
/// `replace`, that preset is replaced in its place (renamed), and presets combining it follow.
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let name = checked_name(req_str(p, "name", SAVE_ID)?).map_err(|e| bad(SAVE_ID, e))?;
    let settings = checked_settings(p.get("settings").unwrap_or(&Value::Null), SAVE_ID)?;
    let replace = match p.get("replace").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => {
            let old = v.as_str().ok_or_else(|| bad(SAVE_ID, "`replace` must be a preset name"))?;
            Some(find(s, old).ok_or_else(|| bad(SAVE_ID, format!("there is no Topaz Gigapixel preset {old:?}")))?.name.clone())
        }
    };
    // The preset written over: the replaced one, else the one of the same name.
    let old = replace.clone().unwrap_or_else(|| name.clone());
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    if replace.is_some() && !same(&old, &name) && find(s, &name).is_some() {
        return Err(bad(SAVE_ID, format!("there is already a Topaz Gigapixel preset called “{name}”")));
    }
    if let Some(names) = settings.get("presets").and_then(Value::as_array) {
        let names: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
        if names.iter().any(|n| same(n, &name) || same(n, &old)) {
            return Err(bad(SAVE_ID, "a preset can't run itself"));
        }
        steps_of(s, &names, SAVE_ID)?;
        // A preset others combine stays one upscale.
        if let Some(user) = s.presets.gigapixel_presets.iter().find(|pr| pr.combined().is_some_and(|c| c.iter().any(|n| same(n, &old)))) {
            return Err(bad(SAVE_ID, format!("“{}” combines this preset, so it must stay a single upscale", user.name)));
        }
    }
    let list = &mut s.presets.gigapixel_presets;
    let at = list.iter().position(|pr| same(&pr.name, &old));
    if at.is_none() && list.len() >= MAX_PRESETS {
        return Err(bad(SAVE_ID, format!("At most {MAX_PRESETS} Topaz Gigapixel presets can be saved")));
    }
    let preset = GigapixelPreset { name: name.clone(), settings };
    match at.and_then(|i| list.get_mut(i)) {
        Some(slot) => *slot = preset.clone(),
        None => list.push(preset.clone()),
    }
    if old != name {
        for pr in list.iter_mut() {
            if let Some(names) = pr.settings.get_mut("presets").and_then(Value::as_array_mut) {
                for n in names.iter_mut().filter(|n| n.as_str().is_some_and(|n| same(n, &old))) {
                    *n = json!(name);
                }
            }
        }
    }
    s.presets_changed();
    Ok(json!({"preset": preset}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = req_str(p, "name", DELETE_ID)?;
    let index = s.presets.gigapixel_presets.iter().position(|pr| pr.name.eq_ignore_ascii_case(name.trim())).ok_or_else(|| bad(DELETE_ID, "No such Topaz Gigapixel preset"))?;
    let removed = s.presets.gigapixel_presets.remove(index);
    s.presets_changed();
    Ok(json!({"deleted": removed.name}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: LIST_ID,
            label: "List Topaz Gigapixel Presets",
            menu: &[],
            shortcut: None,
            params: "{} -> {presets:[{name,settings}],last} (last: the settings of the last upscale Gigapixel's app sent back, or null)",
            enabled: always,
            run: |s, _| Ok(json!({"presets": s.presets.gigapixel_presets, "last": s.presets.gigapixel_last})),
            journal: false,
        },
        CommandSpec {
            id: SAVE_ID,
            label: "Save Topaz Gigapixel Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str,"settings":{"model"?,"modelVersion"?,"scale"|"width"|"height","denoise"?,"sharpen"?,"compression"?,"faceRecovery"?} (one upscale, as file.automate.topazGigapixel takes it) | {"presets":[name,name,…] (2..4 one-upscale presets),"combine":"chain|layers"?},"replace":name? (the preset to replace, renaming it)} -> {preset} (a saved name is replaced)"#,
            enabled: always,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: DELETE_ID,
            label: "Delete Topaz Gigapixel Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str} -> {deleted}"#,
            enabled: always,
            run: delete,
            journal: false,
        },
    ]
}
