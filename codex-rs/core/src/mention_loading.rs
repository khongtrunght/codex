//! Loading content for @ mentioned files and directories

use std::path::Path;

use anyhow::Result;
use codex_protocol::user_input::LineRange;
use tokio::fs;

use crate::mention_extraction::ExtractedMention;
use crate::tools::handlers::read_file::slice;

/// Loaded content for an @ mention
#[derive(Debug, Clone)]
pub enum MentionContent {
    File {
        path: String,
        content: String,
        line_range: Option<LineRange>,
        truncated: bool,
    },
    Directory {
        path: String,
        listing: String,
    },
}

/// Maximum number of lines to include in file content
const MAX_LINES: usize = 2000;

/// Load content for all extracted mentions
pub async fn load_mention_contents(mentions: Vec<ExtractedMention>) -> Vec<MentionContent> {
    let mut contents = Vec::new();

    for mention in mentions {
        // Check if directory at load time instead of storing in ExtractedMention
        let is_directory = mention.absolute_path.is_dir();

        let result = if is_directory {
            load_directory_listing(&mention.absolute_path)
                .await
                .map(|listing| MentionContent::Directory {
                    path: mention.original_path,
                    listing,
                })
        } else {
            load_file_content(&mention.absolute_path, mention.line_range.as_ref())
                .await
                .map(|(content, truncated)| MentionContent::File {
                    path: mention.original_path,
                    content,
                    line_range: mention.line_range,
                    truncated,
                })
        };

        match result {
            Ok(content) => contents.push(content),
            Err(e) => {
                tracing::warn!("Failed to load mention content: {e}");
            }
        }
    }

    contents
}

/// Load directory listing
async fn load_directory_listing(path: &Path) -> Result<String> {
    let mut entries = fs::read_dir(path).await?;
    let mut listing = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Add trailing / for directories
        let display = if entry.file_type().await?.is_dir() {
            format!("{name_str}/")
        } else {
            name_str.to_string()
        };

        listing.push(display);
    }

    listing.sort();
    Ok(listing.join("\n"))
}

/// Load file content with optional line range.
/// Reuses the existing read_file tool's slice::read for consistent behavior.
async fn load_file_content(path: &Path, line_range: Option<&LineRange>) -> Result<(String, bool)> {
    let (offset, limit) = match line_range {
        Some(range) => {
            let start = range.start as usize;
            let limit = match range.end {
                Some(end) => (end - range.start + 1) as usize,
                None => 1, // Single line
            };
            (start, limit)
        }
        None => (1, MAX_LINES),
    };

    let lines = slice::read(path, offset, limit)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Truncated if we hit the limit (for full file reads only)
    let truncated = line_range.is_none() && lines.len() >= MAX_LINES;

    Ok((lines.join("\n"), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs::File;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_load_file_full() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");

        let mut file = File::create(&file_path).await.unwrap();
        file.write_all(b"line 1\nline 2\nline 3\n").await.unwrap();

        let (content, truncated) = load_file_content(&file_path, None).await.unwrap();

        assert!(!truncated);
        assert!(content.contains("L1: line 1"));
        assert!(content.contains("L2: line 2"));
        assert!(content.contains("L3: line 3"));
    }

    #[tokio::test]
    async fn test_load_file_with_range() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");

        let mut file = File::create(&file_path).await.unwrap();
        file.write_all(b"line 1\nline 2\nline 3\nline 4\nline 5\n")
            .await
            .unwrap();

        let range = LineRange {
            start: 2,
            end: Some(4),
        };
        let (content, truncated) = load_file_content(&file_path, Some(&range)).await.unwrap();

        assert!(!truncated);
        assert!(content.contains("L2: line 2"));
        assert!(content.contains("L3: line 3"));
        assert!(content.contains("L4: line 4"));
        assert!(!content.contains("L1: line 1"));
        assert!(!content.contains("L5: line 5"));
    }

    #[tokio::test]
    async fn test_load_directory() {
        let temp_dir = TempDir::new().unwrap();

        // Create some files and a subdirectory
        File::create(temp_dir.path().join("file1.txt"))
            .await
            .unwrap();
        File::create(temp_dir.path().join("file2.rs"))
            .await
            .unwrap();
        fs::create_dir(temp_dir.path().join("subdir"))
            .await
            .unwrap();

        let listing = load_directory_listing(temp_dir.path()).await.unwrap();

        assert!(listing.contains("file1.txt"));
        assert!(listing.contains("file2.rs"));
        assert!(listing.contains("subdir/"));
    }
}
