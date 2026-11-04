//! Photoshop tool presets (`.tpl`).
//!
//! The file has an `8BTP` header, a version, a block count, then aligned `8BIM` resources.
//! The `tptp` resource contains named presets; each preset is a UTF-16 name, a descriptor
//! version and one ActionDescriptor. The descriptor parser is shared with PSD and other
//! Photoshop formats.
//!
//! Layout verified against Photoshop 27.7 files described by TypeR's public MIT reader and
//! test fixtures: `8BTP`, `u32` version, `u32` block count, then `8BIM` blocks. Presets have
//! no length of their own; the descriptor walk delimits them.

use crate::descriptor::{Descriptor, UnicodeString};
use crate::error::{PsdError, Result};

/// Largest accepted tool-preset library.
pub const MAX_FILE_BYTES: usize = 64 << 20;
/// Largest number of presets accepted in one library.
pub const MAX_PRESETS: usize = 10_000;
const MAX_RESOURCES: usize = 10_000;
const MAX_NAME_UNITS: usize = 4096;

/// One Photoshop tool preset and its tool-specific settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPreset {
    /// Name shown in Photoshop's Tool Presets panel.
    pub name: String,
    /// Tool class identifier, such as `typeCreateOrEditTool`.
    pub tool: String,
    /// Tool-specific settings.
    pub settings: Descriptor,
}

/// Parsed `.tpl` library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TplFile {
    /// Presets in file order.
    pub presets: Vec<ToolPreset>,
    /// Resource identifiers not interpreted by this reader.
    pub ignored_resources: Vec<String>,
}

fn take<'a>(data: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = at.checked_add(len).ok_or(PsdError::LimitExceeded("TPL offset overflow"))?;
    let bytes = data.get(*at..end).ok_or(PsdError::UnexpectedEof { offset: *at, needed: end.saturating_sub(data.len()) })?;
    *at = end;
    Ok(bytes)
}

fn read_u32(data: &[u8], at: &mut usize) -> Result<u32> {
    let bytes = take(data, at, 4)?;
    let Some(bytes) = bytes.get(..4) else {
        return Err(PsdError::UnexpectedEof { offset: at.saturating_sub(4), needed: 4 });
    };
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_name(data: &[u8], at: &mut usize) -> Result<String> {
    let units = read_u32(data, at)? as usize;
    if units > MAX_NAME_UNITS {
        return Err(PsdError::LimitExceeded("TPL preset name too long"));
    }
    let byte_len = units.checked_mul(2).ok_or(PsdError::LimitExceeded("TPL name length overflow"))?;
    let bytes = take(data, at, byte_len)?;
    let mut utf16 = Vec::new();
    utf16.try_reserve_exact(units).map_err(|_| PsdError::LimitExceeded("TPL name allocation"))?;
    for pair in bytes.as_chunks::<2>().0 {
        utf16.push(u16::from_be_bytes(*pair));
    }
    Ok(UnicodeString(utf16).to_string_lossy())
}

fn parse_presets(data: &[u8], out: &mut TplFile) -> Result<()> {
    let mut at = 0usize;
    let count = read_u32(data, &mut at)? as usize;
    if count > MAX_PRESETS || out.presets.len().saturating_add(count) > MAX_PRESETS {
        return Err(PsdError::LimitExceeded("too many TPL presets"));
    }
    out.presets.try_reserve(count.min(4096)).map_err(|_| PsdError::LimitExceeded("TPL preset allocation"))?;

    for index in 0..count {
        let name = read_name(data, &mut at)?;
        // Photoshop writes a 32-bit descriptor version here (measured: 16).
        take(data, &mut at, 4)?;
        let property_bytes = data.get(at..).ok_or(PsdError::invalid("invalid TPL descriptor bounds"))?;
        let (settings, used) = Descriptor::parse_prefix(property_bytes)?;
        at = at.checked_add(used).ok_or(PsdError::LimitExceeded("TPL descriptor offset overflow"))?;
        let name = if name.trim().is_empty() { format!("Text Preset {}", index + 1) } else { name };
        let tool = String::from_utf8_lossy(settings.class_id.as_bytes()).into_owned();
        out.presets.push(ToolPreset { name, tool, settings });
    }

    let trailing = data.get(at..).ok_or(PsdError::invalid("invalid TPL preset bounds"))?;
    if trailing.len() > 3 || trailing.iter().any(|byte| *byte != 0) {
        return Err(PsdError::invalid("unexpected trailing data in TPL preset resource"));
    }
    Ok(())
}

/// Parses a Photoshop tool-preset library.
///
/// The parser bounds file size, resource lengths, names, descriptor nesting, and preset counts.
/// Unknown resources are ignored and reported; malformed recognized resources return an error.
pub fn parse(data: &[u8]) -> Result<TplFile> {
    if data.len() > MAX_FILE_BYTES {
        return Err(PsdError::LimitExceeded("TPL file too large"));
    }
    let mut at = 0usize;
    let signature = take(data, &mut at, 4)?;
    if signature != b"8BTP" {
        let mut found = [0u8; 4];
        found.copy_from_slice(signature);
        return Err(PsdError::InvalidSignature { expected: "8BTP", found });
    }
    // Version and block count. The count bounds the walk but Photoshop files may carry
    // trailing blocks; the loop below still stops at the end of the data.
    let _version = read_u32(data, &mut at)?;
    let declared_blocks = read_u32(data, &mut at)? as usize;
    if declared_blocks > MAX_RESOURCES {
        return Err(PsdError::LimitExceeded("too many TPL resources"));
    }

    let mut out = TplFile::default();
    let mut resource_count = 0usize;
    while at < data.len() {
        let remaining = data.get(at..).ok_or(PsdError::invalid("invalid TPL resource offset"))?;
        if remaining.iter().all(|byte| *byte == 0) {
            break;
        }
        resource_count = resource_count.checked_add(1).ok_or(PsdError::LimitExceeded("too many TPL resources"))?;
        if resource_count > MAX_RESOURCES {
            return Err(PsdError::LimitExceeded("too many TPL resources"));
        }
        let signature = take(data, &mut at, 4)?;
        if signature != b"8BIM" {
            let mut found = [0u8; 4];
            found.copy_from_slice(signature);
            return Err(PsdError::InvalidSignature { expected: "8BIM resource", found });
        }
        let key = take(data, &mut at, 4)?;
        let len = read_u32(data, &mut at)? as usize;
        let section = take(data, &mut at, len)?;
        if key == b"tptp" {
            parse_presets(section, &mut out)?;
        } else {
            out.ignored_resources.push(String::from_utf8_lossy(key).into_owned());
        }
        // Alignment is relative to the start of the file, not the payload length.
        let padding = (4 - at % 4) % 4;
        if padding > 0 {
            let pad = take(data, &mut at, padding)?;
            if pad.iter().any(|byte| *byte != 0) {
                return Err(PsdError::invalid("non-zero TPL resource padding"));
            }
        }
    }
    if out.presets.is_empty() {
        return Err(PsdError::invalid("the TPL file contains no tool presets"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Value;

    fn put_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn name(out: &mut Vec<u8>, value: &str) {
        let units: Vec<u16> = value.encode_utf16().collect();
        put_u32(out, units.len() as u32);
        for unit in units {
            out.extend_from_slice(&unit.to_be_bytes());
        }
    }

    fn append_entry(section: &mut Vec<u8>, name_value: &str, descriptor: Descriptor) {
        name(section, name_value);
        put_u32(section, 16);
        section.extend(descriptor.to_bytes());
    }

    fn sample() -> Vec<u8> {
        let character = Descriptor::new("TxtS")
            .with("fontPostScriptName", Value::Text(UnicodeString::new_nul("ExampleSans-Bold")))
            .with("FntN", Value::Text(UnicodeString::new_nul("Example Sans")))
            .with("Sz  ", Value::UnitFloat { unit: *b"#Pnt", value: 18.0 })
            .with(
                "Clr ",
                Value::Descriptor(
                    Descriptor::new("RGBC")
                        .with("Rd  ", Value::Double(200.0))
                        .with("Grn ", Value::Double(30.0))
                        .with("Bl  ", Value::Double(10.0)),
                ),
            );
        let settings = Descriptor::new("typeCreateOrEditTool")
            .with("textToolCharacterOptions", Value::Descriptor(Descriptor::new("null").with("TxtS", Value::Descriptor(character))));
        let brush = Descriptor::new("paintbrushTool").with("Sz  ", Value::Integer(12));

        let mut section = Vec::new();
        put_u32(&mut section, 2);
        append_entry(&mut section, "Body", settings);
        append_entry(&mut section, "$$$/Presets/Tools/Brush=Brush", brush);

        let mut file = b"8BTP".to_vec();
        put_u32(&mut file, 2);
        put_u32(&mut file, 1);
        file.extend_from_slice(b"8BIMtptp");
        put_u32(&mut file, section.len() as u32);
        file.extend(section);
        while !file.len().is_multiple_of(4) {
            file.push(0);
        }
        file
    }

    #[test]
    fn parses_text_and_non_text_tool_presets() {
        let parsed = parse(&sample()).unwrap();
        assert_eq!(parsed.presets.len(), 2);
        assert_eq!(parsed.presets[0].name, "Body");
        assert_eq!(parsed.presets[0].tool, "typeCreateOrEditTool");
        assert!(matches!(parsed.presets[0].settings.get("textToolCharacterOptions"), Some(Value::Descriptor(_))));
        assert_eq!(parsed.presets[1].tool, "paintbrushTool");
    }

    #[test]
    fn rejects_invalid_and_truncated_tpl_files() {
        let bytes = sample();
        assert!(parse(b"").is_err());
        assert!(parse(b"8BPS............").is_err());
        // The header alone and a header cut before its descriptor cannot be a preset.
        for end in 0..=16 {
            assert!(parse(&bytes[..end.min(bytes.len())]).is_err(), "accepted truncation at byte {end}");
        }
        assert!(parse(&bytes[..bytes.len() - 4]).is_err(), "accepted a preset cut inside its descriptor");
    }

    #[test]
    fn rejects_property_count_that_exceeds_resource_data() {
        let mut bytes = sample();
        let count = bytes.windows(8).position(|window| window == b"8BIMtptp").unwrap() + 12;
        let name_units = u32::from_be_bytes(bytes.get(count..count + 4).unwrap().try_into().unwrap()) as usize;
        let descriptor_at = count + 4 + name_units * 2 + 4;
        // Descriptor: name (0 units), class id (0 + 4 bytes), then the property count.
        let property_count_at = descriptor_at + 4 + 4 + 4;
        bytes[property_count_at..property_count_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn rejects_excessive_resource_count() {
        let mut bytes = b"8BTP".to_vec();
        put_u32(&mut bytes, 2);
        put_u32(&mut bytes, 0);
        for _ in 0..=MAX_RESOURCES {
            bytes.extend_from_slice(b"8BIMjunk");
            put_u32(&mut bytes, 0);
        }
        assert!(parse(&bytes).is_err());
    }
}
