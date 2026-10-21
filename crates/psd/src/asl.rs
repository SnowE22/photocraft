//! Photoshop layer-style libraries (`.asl`).
//!
//! ASL version 3 stores each named style as two ActionDescriptors: metadata and a `Styl`
//! descriptor containing the layer effects. Descriptor parsing, including nesting and value
//! limits, is shared with PSD files.

use crate::descriptor::{Descriptor, Value, VersionedDescriptor};
use crate::error::{PsdError, Result};

/// Largest accepted number of styles in one file.
pub const MAX_STYLES: usize = 10_000;
/// Largest accepted encoded record for one style.
pub const MAX_STYLE_BYTES: usize = 16 << 20;
/// Largest accepted ASL file.
pub const MAX_FILE_BYTES: usize = 512 << 20;

/// One named style and its original Photoshop style descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct AslStyle {
    /// The name stored in the ASL metadata descriptor.
    pub name: String,
    /// Photoshop's style identifier, when present.
    pub id: Option<String>,
    /// The `Styl` ActionDescriptor, including the nested `Lefx` effects descriptor.
    pub descriptor: Descriptor,
}

/// Parsed styles and opaque Photoshop resource blocks that follow them.
#[derive(Clone, Debug, PartialEq)]
pub struct AslFile {
    /// Named styles stored in the file.
    pub styles: Vec<AslStyle>,
    /// Four-byte identifiers of trailing resource blocks not interpreted by this parser.
    pub ignored_resources: Vec<String>,
}

fn be_u16(data: &[u8], at: usize) -> Result<u16> {
    let b = data.get(at..at.saturating_add(2)).ok_or(PsdError::UnexpectedEof { offset: at, needed: 2 })?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn be_u32(data: &[u8], at: usize) -> Result<u32> {
    let b = data.get(at..at.saturating_add(4)).ok_or(PsdError::UnexpectedEof { offset: at, needed: 4 })?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn text(d: &Descriptor, key: &str) -> Option<String> {
    match d.get(key)? {
        Value::Text(value) => {
            let s = value.to_string_lossy();
            let s = s.trim_end_matches('\0').trim();
            (!s.is_empty()).then(|| s.to_owned())
        }
        _ => None,
    }
}

/// Parses a Photoshop `.asl` layer-style library (signature `8BSL`, ASL version 3).
///
/// Files with unsupported versions, truncated records, excessive sizes/counts, malformed
/// descriptors, or trailing data return an error rather than a partial import.
pub fn parse(data: &[u8]) -> Result<AslFile> {
    if data.len() > MAX_FILE_BYTES {
        return Err(PsdError::LimitExceeded("ASL file too large"));
    }
    let header = data.get(..16).ok_or(PsdError::UnexpectedEof { offset: 0, needed: 16 })?;
    let signature = header.get(2..6).ok_or(PsdError::UnexpectedEof { offset: 2, needed: 4 })?;
    if signature != b"8BSL" {
        let mut found = [0u8; 4];
        found.copy_from_slice(signature);
        return Err(PsdError::InvalidSignature { expected: "8BSL", found });
    }
    let file_version = be_u16(data, 0)?;
    if file_version != 2 {
        return Err(PsdError::Unsupported(format!("ASL file version {file_version} (expected 2)")));
    }
    let asl_version = be_u16(data, 6)?;
    if asl_version != 3 {
        return Err(PsdError::Unsupported(format!("ASL version {asl_version} (only version 3 is supported)")));
    }
    let count = be_u32(data, 12)? as usize;
    if count > MAX_STYLES {
        return Err(PsdError::LimitExceeded("too many ASL styles"));
    }
    let minimum_records = count.checked_mul(4).ok_or(PsdError::LimitExceeded("ASL style count overflow"))?;
    if minimum_records > data.len().saturating_sub(16) {
        return Err(PsdError::UnexpectedEof { offset: 16, needed: minimum_records });
    }

    let mut styles = Vec::with_capacity(count.min(4096));
    let mut at = 16usize;
    for index in 0..count {
        let len = be_u32(data, at)? as usize;
        at = at.checked_add(4).ok_or(PsdError::LimitExceeded("ASL offset overflow"))?;
        if len > MAX_STYLE_BYTES {
            return Err(PsdError::LimitExceeded("ASL style record too large"));
        }
        let end = at.checked_add(len).ok_or(PsdError::LimitExceeded("ASL offset overflow"))?;
        let record = data.get(at..end).ok_or(PsdError::UnexpectedEof { offset: at, needed: len })?;
        let (metadata, meta_len) = VersionedDescriptor::parse_prefix(record)?;
        let (style, style_len) = VersionedDescriptor::parse_prefix(record.get(meta_len..).ok_or(PsdError::UnexpectedEof { offset: at, needed: len })?)?;
        let consumed = meta_len.checked_add(style_len).ok_or(PsdError::LimitExceeded("ASL descriptor offset overflow"))?;
        let padding = record.get(consumed..).ok_or(PsdError::UnexpectedEof { offset: at, needed: len })?;
        if padding.len() > 3 || padding.iter().any(|byte| *byte != 0) {
            return Err(PsdError::invalid(format!("style record {index} has invalid trailing data")));
        }
        if style.descriptor.class_id.as_bytes() != b"Styl" {
            return Err(PsdError::invalid(format!("style record {index} has no Styl descriptor")));
        }
        let name = text(&metadata.descriptor, "Nm  ").unwrap_or_else(|| format!("Style {}", index + 1));
        let id = text(&metadata.descriptor, "Idnt");
        styles.push(AslStyle { name, id, descriptor: style.descriptor });
        at = end;
    }
    let mut ignored_resources = Vec::new();
    while at < data.len() {
        let header = data.get(at..at.saturating_add(12)).ok_or(PsdError::UnexpectedEof { offset: at, needed: 12 })?;
        if header.get(..4) != Some(b"8BIM") {
            return Err(PsdError::invalid("unexpected data after ASL styles"));
        }
        let key = header.get(4..8).ok_or(PsdError::UnexpectedEof { offset: at.saturating_add(4), needed: 4 })?;
        let len = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
        let start = at.checked_add(12).ok_or(PsdError::LimitExceeded("ASL resource offset overflow"))?;
        let end = start.checked_add(len).ok_or(PsdError::LimitExceeded("ASL resource offset overflow"))?;
        data.get(start..end).ok_or(PsdError::UnexpectedEof { offset: start, needed: len })?;
        ignored_resources.push(String::from_utf8_lossy(key).into_owned());
        at = end;
    }
    if styles.is_empty() {
        return Err(PsdError::invalid("the ASL file contains no styles"));
    }
    Ok(AslFile { styles, ignored_resources })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::UnicodeString;

    fn sample() -> Vec<u8> {
        let metadata = Descriptor::new("null")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Sample style")))
            .with("Idnt", Value::Text(UnicodeString::new_nul("style-id")));
        let style = Descriptor::new("Styl").with("Lefx", Value::Descriptor(Descriptor::new("Lefx")));
        let mut record = VersionedDescriptor::new(metadata).to_bytes();
        record.extend(VersionedDescriptor::new(style).to_bytes());
        let mut file = vec![0, 2];
        file.extend_from_slice(b"8BSL");
        file.extend_from_slice(&3u16.to_be_bytes());
        file.extend_from_slice(&0u32.to_be_bytes());
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(&(record.len() as u32).to_be_bytes());
        file.extend(record);
        file
    }

    #[test]
    fn parses_named_asl_styles() {
        let file = parse(&sample()).unwrap();
        assert_eq!(file.styles.len(), 1);
        assert_eq!(file.styles[0].name, "Sample style");
        assert_eq!(file.styles[0].id.as_deref(), Some("style-id"));
        assert_eq!(file.styles[0].descriptor.class_id.as_bytes(), b"Styl");
        assert!(file.ignored_resources.is_empty());
    }

    #[test]
    fn rejects_truncation_bad_versions_and_excess_counts() {
        let bytes = sample();
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end]).is_err(), "accepted truncation at {end}");
        }
        let mut bad_version = bytes.clone();
        bad_version[6..8].copy_from_slice(&4u16.to_be_bytes());
        assert!(parse(&bad_version).is_err());
        let mut excessive = bytes;
        excessive[12..16].copy_from_slice(&((MAX_STYLES as u32) + 1).to_be_bytes());
        assert!(parse(&excessive).is_err());
    }

    #[test]
    fn rejects_wrong_signature_and_trailing_data() {
        assert!(parse(b"").is_err());
        assert!(parse(b"8BPS............").is_err());
        let mut bytes = sample();
        bytes.push(0);
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn accepts_only_small_zero_padding_inside_style_records() {
        let mut bytes = sample();
        let len = be_u32(&bytes, 16).unwrap();
        bytes[16..20].copy_from_slice(&(len + 1).to_be_bytes());
        bytes.push(0);
        assert_eq!(parse(&bytes).unwrap().styles.len(), 1);
        *bytes.last_mut().unwrap() = 1;
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn validates_and_reports_trailing_resource_blocks() {
        let mut bytes = sample();
        let resource_at = bytes.len();
        bytes.extend_from_slice(b"8BIMphry");
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes.extend_from_slice(&[1, 2, 3]);
        let file = parse(&bytes).unwrap();
        assert_eq!(file.ignored_resources, ["phry"]);
        let mut invalid = bytes.clone();
        invalid[resource_at] = b'X';
        assert!(parse(&invalid).is_err());
        let mut truncated = sample();
        truncated.extend_from_slice(b"8BIMphry");
        truncated.extend_from_slice(&4u32.to_be_bytes());
        truncated.extend_from_slice(&[1, 2, 3]);
        assert!(parse(&truncated).is_err());
    }
}
