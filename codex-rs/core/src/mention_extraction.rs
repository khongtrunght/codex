//! Extraction and parsing of @ mentions from user input
//!
//! File mentions are detected by parsing `@path` patterns from the raw text,
//! following the same approach as Claude JS (backend-only regex parsing).

use std::path::Path;
use std::path::PathBuf;

use codex_protocol::user_input::LineRange;
use codex_protocol::user_input::UserInput;
use regex::Regex;

/// Extracted mention from user text
#[derive(Debug, Clone)]
pub struct ExtractedMention {
    /// Resolved absolute path to the file/directory
    pub absolute_path: PathBuf,
    /// Original relative path from the mention
    pub original_path: String,
    /// Optional line range (#L syntax)
    pub line_range: Option<LineRange>,
}

/// Result of mention extraction
pub struct MentionExtractionResult {
    pub mentions: Vec<ExtractedMention>,
    pub warnings: Vec<String>,
}

/// Extract all @ mentions from UserInput items by parsing @path patterns from text.
pub fn extract_mentions(inputs: &[UserInput], cwd: &Path) -> MentionExtractionResult {
    let mut mentions = Vec::new();
    let mut warnings = Vec::new();

    for input in inputs {
        if let UserInput::Text { text, .. } = input {
            // Parse @mentions from raw text using regex
            let (extracted, extracted_warnings) = extract_from_text(text, cwd);
            mentions.extend(extracted);
            warnings.extend(extracted_warnings);
        }
    }

    // Deduplicate by absolute path
    mentions.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
    mentions.dedup_by(|a, b| a.absolute_path == b.absolute_path);

    MentionExtractionResult { mentions, warnings }
}

/// Resolve a single mention to an absolute path
fn resolve_mention(
    path: &str,
    line_range: Option<LineRange>,
    cwd: &Path,
) -> Result<ExtractedMention, String> {
    // Expand ~ if present
    let expanded_path = if path.starts_with('~') {
        let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
        if path == "~" {
            home
        } else if let Some(stripped) = path.strip_prefix("~/") {
            home.join(stripped)
        } else {
            return Err("Unsupported ~ syntax (only ~ and ~/ supported)".to_string());
        }
    } else if path.starts_with('/') {
        // Absolute path
        PathBuf::from(path)
    } else {
        // Relative path from cwd
        cwd.join(path)
    };

    // Check if path exists
    let absolute_path = expanded_path
        .canonicalize()
        .map_err(|_| format!("@ mention path does not exist: {path}"))?;

    Ok(ExtractedMention {
        absolute_path,
        original_path: path.to_string(),
        line_range,
    })
}

/// Extract mentions from text by parsing @path patterns.
/// Supports both quoted (@"path with spaces") and unquoted (@path/to/file) forms.
fn extract_from_text(text: &str, cwd: &Path) -> (Vec<ExtractedMention>, Vec<String>) {
    let mut mentions = Vec::new();
    let mut warnings = Vec::new();

    // Find all @mentions in text
    // Quoted: @"path with spaces"
    // Unquoted: @path/to/file
    let quoted_re = Regex::new(r#"@"([^"]+)""#).expect("valid regex");
    let unquoted_re = Regex::new(r"@([^\s]+)").expect("valid regex");

    for cap in quoted_re.captures_iter(text) {
        let path_str = cap.get(1).expect("capture group exists").as_str();
        let (path, line_range) = parse_line_range_from_path(path_str);

        match resolve_mention(&path, line_range, cwd) {
            Ok(mention) => mentions.push(mention),
            Err(warning) => {
                tracing::warn!("{warning}");
                warnings.push(warning);
            }
        }
    }

    for cap in unquoted_re.captures_iter(text) {
        let full_match = cap.get(0).expect("match exists");
        let byte_start = full_match.start();

        // Skip if this is part of a quoted mention (already handled above)
        if byte_start > 0 && text[..byte_start].ends_with('"') {
            continue;
        }

        let path_str = cap.get(1).expect("capture group exists").as_str();
        let (path, line_range) = parse_line_range_from_path(path_str);

        match resolve_mention(&path, line_range, cwd) {
            Ok(mention) => mentions.push(mention),
            Err(warning) => {
                tracing::warn!("{warning}");
                warnings.push(warning);
            }
        }
    }

    (mentions, warnings)
}

/// Parse line range from path string (helper for text extraction)
fn parse_line_range_from_path(path: &str) -> (String, Option<LineRange>) {
    // Pattern: path#L(start)(-(end))?
    if let Some(hash_pos) = path.find("#L") {
        let path_part = &path[..hash_pos];
        let range_part = &path[hash_pos + 2..]; // Skip "#L"

        // Parse start-end or just start
        let (start_str, end_str) = match range_part.find('-') {
            Some(dash_pos) => (&range_part[..dash_pos], Some(&range_part[dash_pos + 1..])),
            None => (range_part, None),
        };

        // Parse start line number
        if let Ok(start) = start_str.parse::<u32>() {
            let end = end_str.and_then(|s| s.parse::<u32>().ok());
            return (path_part.to_string(), Some(LineRange { start, end }));
        }
    }

    (path.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_range_single_line() {
        let (path, range) = parse_line_range_from_path("file.rs#L42");
        assert_eq!(path, "file.rs");
        assert_eq!(
            range,
            Some(LineRange {
                start: 42,
                end: None
            })
        );
    }

    #[test]
    fn test_parse_line_range_range() {
        let (path, range) = parse_line_range_from_path("file.rs#L10-20");
        assert_eq!(path, "file.rs");
        assert_eq!(
            range,
            Some(LineRange {
                start: 10,
                end: Some(20)
            })
        );
    }

    #[test]
    fn test_parse_line_range_no_range() {
        let (path, range) = parse_line_range_from_path("file.rs");
        assert_eq!(path, "file.rs");
        assert_eq!(range, None);
    }

    #[test]
    fn test_parse_line_range_invalid_syntax() {
        let (path, range) = parse_line_range_from_path("file.rs#L");
        assert_eq!(path, "file.rs#L");
        assert_eq!(range, None);
    }
}
