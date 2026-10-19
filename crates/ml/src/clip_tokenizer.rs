//! CLIP's byte-level BPE tokenizer (Radford et al. 2021), as Stable Diffusion 1.x's text encoder
//! reads prompts: lower-cased text split into words, numbers and punctuation, each word's UTF-8
//! bytes mapped to printable characters, merged by the ranked pairs in `merges.txt`, then looked up
//! in `vocab.json`; start and end tokens around, padded with the end token to 77.
//!
//! Written from the published algorithm (`simple_tokenizer.py`), without its `ftfy` text repair.

use std::collections::HashMap;

use crate::MlError;

/// Tokens the text encoder reads, start and end included.
pub const LENGTH: usize = 77;

/// Merges after the header that CLIP uses (`49152 - 256 - 2`).
const MERGES: usize = 48_894;

pub struct Tokenizer {
    vocab: HashMap<String, i64>,
    ranks: HashMap<(String, String), usize>,
    bytes: [char; 256],
    start: i64,
    end: i64,
}

/// GPT-2's map from bytes to printable characters: printable Latin-1 bytes stand for themselves,
/// the rest are moved past 255.
fn byte_chars() -> [char; 256] {
    let mut out = ['\0'; 256];
    let mut extra = 0u32;
    for (b, slot) in out.iter_mut().enumerate() {
        let printable = (0x21..=0x7e).contains(&b) || (0xa1..=0xac).contains(&b) || (0xae..=0xff).contains(&b);
        let code = if printable {
            b as u32
        } else {
            extra += 1;
            255 + extra
        };
        *slot = char::from_u32(code).unwrap_or('?');
    }
    out
}

impl Tokenizer {
    /// From the contents of `vocab.json` and `merges.txt`.
    pub fn new(vocab_json: &str, merges_txt: &str) -> Result<Self, MlError> {
        let raw: serde_json::Map<String, serde_json::Value> = serde_json::from_str(vocab_json).map_err(|e| MlError::Load(format!("vocab.json: {e}")))?;
        let vocab: HashMap<String, i64> = raw.into_iter().filter_map(|(k, v)| v.as_i64().map(|v| (k, v))).collect();
        let mut ranks = HashMap::new();
        for (i, line) in merges_txt.lines().skip(1).take(MERGES).enumerate() {
            let mut parts = line.split(' ');
            if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
                ranks.insert((a.to_string(), b.to_string()), i);
            }
        }
        let start = vocab.get("<|startoftext|>").copied().ok_or_else(|| MlError::Load("vocab.json has no <|startoftext|>".into()))?;
        let end = vocab.get("<|endoftext|>").copied().ok_or_else(|| MlError::Load("vocab.json has no <|endoftext|>".into()))?;
        if ranks.is_empty() {
            return Err(MlError::Load("merges.txt has no merges".into()));
        }
        Ok(Self { vocab, ranks, bytes: byte_chars(), start, end })
    }

    /// `text` as [`LENGTH`] token ids: start, the text's tokens (cut to fit), end, then end tokens.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        let mut ids = vec![self.start];
        for word in split(&text.to_lowercase()) {
            let mapped: String = word.bytes().map(|b| self.bytes[usize::from(b)]).collect();
            for piece in self.bpe(&mapped) {
                if let Some(id) = self.vocab.get(&piece) {
                    ids.push(*id);
                }
            }
        }
        ids.truncate(LENGTH - 1);
        ids.push(self.end);
        ids.resize(LENGTH, self.end);
        ids
    }

    /// One word's BPE pieces (the last one ends in `</w>`).
    fn bpe(&self, word: &str) -> Vec<String> {
        let chars: Vec<char> = word.chars().collect();
        let Some((last, rest)) = chars.split_last() else { return Vec::new() };
        let mut parts: Vec<String> = rest.iter().map(|c| c.to_string()).collect();
        parts.push(format!("{last}</w>"));
        while parts.len() > 1 {
            // The lowest-ranked adjacent pair.
            let mut best: Option<(usize, usize)> = None;
            for (i, p) in parts.windows(2).enumerate() {
                if let Some(r) = self.ranks.get(&(p[0].clone(), p[1].clone()))
                    && best.is_none_or(|(br, _)| *r < br)
                {
                    best = Some((*r, i));
                }
            }
            let Some((_, at)) = best else { break };
            let (a, b) = (parts[at].clone(), parts[at + 1].clone());
            // Merge every occurrence of it, left to right.
            let mut merged = Vec::with_capacity(parts.len());
            let mut i = 0;
            while i < parts.len() {
                if i + 1 < parts.len() && parts[i] == a && parts[i + 1] == b {
                    merged.push(format!("{a}{b}"));
                    i += 2;
                } else {
                    merged.push(parts[i].clone());
                    i += 1;
                }
            }
            parts = merged;
        }
        parts
    }
}

/// CLIP's pre-tokenizer: `'s 't 're 've 'm 'll 'd`, runs of letters, single digits, runs of other
/// non-space characters; whitespace separates.
fn split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' {
            let rest: String = chars[i + 1..chars.len().min(i + 3)].iter().collect();
            if let Some(s) = ["ll", "re", "ve"].iter().find(|s| rest.starts_with(**s)) {
                out.push(format!("'{s}"));
                i += 3;
                continue;
            }
            if let Some(s) = ["s", "t", "m", "d"].iter().find(|s| rest.starts_with(**s)) {
                out.push(format!("'{s}"));
                i += 2;
                continue;
            }
        }
        let start = i;
        if c.is_alphabetic() {
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
        } else if c.is_numeric() {
            i += 1;
        } else {
            while i < chars.len() && !chars[i].is_whitespace() && !chars[i].is_alphabetic() && !chars[i].is_numeric() {
                i += 1;
            }
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pre_tokenizer_splits_like_clip() {
        assert_eq!(split("a photo, of 2 cats' tails!! it's"), ["a", "photo", ",", "of", "2", "cats", "'", "tails", "!!", "it", "'s"]);
        assert_eq!(split("12"), ["1", "2"]);
        assert!(split("   ").is_empty());
    }

    #[test]
    fn bytes_map_to_256_distinct_printable_characters() {
        let m = byte_chars();
        let set: std::collections::HashSet<char> = m.iter().copied().collect();
        assert_eq!(set.len(), 256);
        assert_eq!(m[usize::from(b'a')], 'a');
        assert_eq!(m[usize::from(b' ')], '\u{120}');
    }

    /// A toy vocabulary: merges build "low" and "er</w>".
    #[test]
    fn words_merge_by_rank_and_pad_to_77() {
        let vocab = r#"{"<|startoftext|>": 0, "<|endoftext|>": 1, "low": 2, "er</w>": 3, "low</w>": 4, "l": 5, "o": 6, "w": 7, "e": 8, "r</w>": 9}"#;
        let merges = "#version: 0.2\nl o\nlo w\ne r</w>\nlow </w>\n";
        let t = Tokenizer::new(vocab, merges).unwrap();
        let ids = t.encode("Lower");
        assert_eq!(&ids[..4], [0, 2, 3, 1]);
        assert_eq!(ids.len(), LENGTH);
        assert!(ids[4..].iter().all(|i| *i == 1));
        // Long text is cut to fit, ending in the end token.
        let long = t.encode(&"lower ".repeat(100));
        assert_eq!(long.len(), LENGTH);
        assert_eq!(long[LENGTH - 1], 1);
        assert!(Tokenizer::new("{}", merges).is_err());
        assert!(Tokenizer::new("not json", merges).is_err());
    }
}
