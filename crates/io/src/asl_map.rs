//! Photoshop ASL layer-style libraries → PhotoCraft effects.
//!
//! The file parser lives in `photocraft-psd`; effect fields use the same descriptor mapper as
//! PSD `lfx2` layer effects. Unsupported effect keys are reported instead of silently discarded.

use photocraft_doc::Effect;
use photocraft_psd::descriptor::Value;

/// One usable named style from an ASL library.
#[derive(Clone, Debug, PartialEq)]
pub struct AslStyle {
    /// Imported style name.
    pub name: String,
    /// Layer effects PhotoCraft can apply.
    pub effects: Vec<Effect>,
}

/// Mapped styles and notes about content that PhotoCraft cannot represent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AslImport {
    /// Styles with at least one supported effect.
    pub styles: Vec<AslStyle>,
    /// Unsupported effects and skipped styles.
    pub warnings: Vec<String>,
}

fn warn(warnings: &mut Vec<String>, message: String) {
    if warnings.iter().any(|warning| warning == &message) {
        return;
    }
    if warnings.len() < 49 {
        warnings.push(message);
    } else if warnings.len() == 49 {
        warnings.push("additional import notes omitted".into());
    }
}

/// Parse and map an ASL library. Styles containing no supported effects are skipped.
pub fn read_asl(bytes: &[u8]) -> Result<AslImport, String> {
    let parsed = photocraft_psd::asl::parse(bytes).map_err(|e| format!("not a readable Photoshop style file (.asl): {e}"))?;
    let mut styles = Vec::with_capacity(parsed.styles.len());
    let mut warnings = Vec::new();
    for key in &parsed.ignored_resources {
        let warning = if key == "phry" { "Photoshop style hierarchy is not imported".to_string() } else { format!("ASL resource `{key}` was not imported") };
        warn(&mut warnings, warning);
    }
    let mut skipped = 0usize;
    for style in parsed.styles {
        let lefx = match style.descriptor.get("Lefx") {
            Some(Value::Descriptor(d) | Value::GlobalObject(d)) => d,
            _ => {
                warn(&mut warnings, format!("{}: missing layer-effects data; skipped", style.name));
                skipped = skipped.saturating_add(1);
                continue;
            }
        };
        let (master, mut effects, unsupported) = crate::effects_map::parse_effects_descriptor(lefx);
        for warning in unsupported {
            warn(&mut warnings, format!("{}: {warning}", style.name));
        }
        if !master {
            for effect in &mut effects {
                effect.set_enabled(false);
            }
            warn(&mut warnings, format!("{}: the master effects switch is off", style.name));
        }
        if effects.is_empty() {
            warn(&mut warnings, format!("{}: no supported effects; skipped", style.name));
            skipped = skipped.saturating_add(1);
            continue;
        }
        styles.push(AslStyle { name: style.name, effects });
    }
    if skipped > 0 {
        warn(&mut warnings, format!("{skipped} styles without supported effects were skipped"));
    }
    if styles.is_empty() {
        let detail = if warnings.is_empty() { String::new() } else { format!(": {}", warnings.join("; ")) };
        return Err(format!("the ASL file contains no importable styles{detail}"));
    }
    Ok(AslImport { styles, warnings })
}

#[cfg(test)]
mod tests {
    use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value, VersionedDescriptor};

    use super::*;

    fn file(master: bool) -> Vec<u8> {
        let metadata = Descriptor::new("null").with("Nm  ", Value::Text(UnicodeString::new_nul("Outlined")));
        let effects = Descriptor::new("Lefx")
            .with("masterFXSwitch", Value::Boolean(master))
            .with("FrFX", Value::Descriptor(Descriptor::new("FrFX")))
            .with("futureFx", Value::Descriptor(Descriptor::new("futureFx")));
        let style = Descriptor::new("Styl").with("Lefx", Value::Descriptor(effects));
        let mut record = VersionedDescriptor::new(metadata).to_bytes();
        record.extend(VersionedDescriptor::new(style).to_bytes());
        let mut bytes = vec![0, 2];
        bytes.extend_from_slice(b"8BSL");
        bytes.extend_from_slice(&3u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&(record.len() as u32).to_be_bytes());
        bytes.extend(record);
        bytes
    }

    #[test]
    fn maps_supported_effects_and_reports_unknown_keys() {
        let imported = read_asl(&file(true)).unwrap();
        assert_eq!(imported.styles.len(), 1);
        assert_eq!(imported.styles[0].name, "Outlined");
        assert_eq!(imported.styles[0].effects.len(), 1);
        assert!(imported.warnings.iter().any(|w| w.contains("futureFx")));
    }

    #[test]
    fn disabled_master_switch_is_preserved_and_invalid_files_fail() {
        let imported = read_asl(&file(false)).unwrap();
        assert!(!imported.styles[0].effects[0].enabled());
        assert!(imported.warnings.iter().any(|w| w.contains("master effects switch is off")));
        assert!(read_asl(&file(true)[..20]).is_err());
    }
}
