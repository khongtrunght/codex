//! Session-level file read state tracking.
//!
//! Tracks files that have been read during the session to:
//! 1. Validate that files are read before being edited/written
//! 2. Detect external modifications via mtime comparison
//! 3. Cache content for compaction recovery

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Information about a file that was read during the session.
#[derive(Debug, Clone)]
pub struct ReadFileInfo {
    /// File modification time when the file was read.
    pub mtime: SystemTime,
    /// File content when read (used for compaction recovery).
    pub content: String,
    /// Line offset if this was a partial read.
    pub offset: Option<usize>,
    /// Line limit if this was a partial read.
    pub limit: Option<usize>,
}

/// Session-level tracking of files that have been read.
///
/// This enables:
/// - Validation that files are read before edit/write
/// - Detection of external modifications (by user, linter, etc.)
/// - Content caching for compaction recovery
#[derive(Debug, Default)]
pub struct ReadFileState {
    /// Map of canonicalized file path to read info.
    files: HashMap<PathBuf, ReadFileInfo>,
}

/// Error returned when file validation fails.
#[derive(Debug, Clone)]
pub enum FileValidationError {
    /// File has not been read yet.
    NotRead { path: PathBuf },
    /// File was modified externally since it was read.
    ModifiedSinceRead { path: PathBuf },
}

impl std::fmt::Display for FileValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileValidationError::NotRead { path } => {
                write!(
                    f,
                    "You must read the file before editing it. Use read_file on '{}' first.",
                    path.display()
                )
            }
            FileValidationError::ModifiedSinceRead { path } => {
                write!(
                    f,
                    "File '{}' has been modified since read, either by the user or by a linter. \
                     Read it again before attempting to write it.",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for FileValidationError {}

impl ReadFileState {
    /// Create a new empty ReadFileState.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a file has been read.
    ///
    /// # Arguments
    /// * `path` - The file path (will be canonicalized)
    /// * `content` - The file content that was read
    /// * `offset` - Optional line offset for partial reads
    /// * `limit` - Optional line limit for partial reads
    pub fn record_read(
        &mut self,
        path: &Path,
        content: String,
        offset: Option<usize>,
        limit: Option<usize>,
    ) {
        let canonical = match dunce::canonicalize(path) {
            Ok(p) => p,
            Err(_) => return, // Silently fail if canonicalization fails
        };

        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now());

        self.files.insert(
            canonical,
            ReadFileInfo {
                mtime,
                content,
                offset,
                limit,
            },
        );
    }

    /// Check if a file has been read and is still valid for editing.
    ///
    /// Returns Ok(()) if the file can be edited, or an error describing
    /// why it cannot.
    ///
    /// # Arguments
    /// * `path` - The file path to validate
    /// * `check_mtime` - If true, also verify the file hasn't been modified
    pub fn validate_for_edit(
        &self,
        path: &Path,
        check_mtime: bool,
    ) -> Result<(), FileValidationError> {
        let canonical = match dunce::canonicalize(path) {
            Ok(p) => p,
            Err(_) => {
                return Err(FileValidationError::NotRead {
                    path: path.to_path_buf(),
                });
            }
        };

        let info = self.files.get(&canonical).ok_or_else(|| {
            FileValidationError::NotRead {
                path: path.to_path_buf(),
            }
        })?;

        if check_mtime {
            if let Ok(metadata) = std::fs::metadata(path) {
                if let Ok(current_mtime) = metadata.modified() {
                    if current_mtime > info.mtime {
                        return Err(FileValidationError::ModifiedSinceRead {
                            path: path.to_path_buf(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if a file has been read (without mtime validation).
    #[allow(dead_code)]
    pub fn was_file_read(&self, path: &Path) -> bool {
        if let Ok(canonical) = dunce::canonicalize(path) {
            self.files.contains_key(&canonical)
        } else {
            false
        }
    }

    /// Get the cached content for a file, if available.
    #[allow(dead_code)]
    pub fn get_content(&self, path: &Path) -> Option<&str> {
        let canonical = dunce::canonicalize(path).ok()?;
        self.files.get(&canonical).map(|info| info.content.as_str())
    }

    /// Update the mtime for a file after a successful write.
    ///
    /// This should be called after edit_file or write_file succeeds.
    pub fn update_after_write(&mut self, path: &Path, new_content: String) {
        let canonical = match dunce::canonicalize(path) {
            Ok(p) => p,
            Err(_) => return,
        };

        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now());

        // Update existing entry or insert new one
        if let Some(info) = self.files.get_mut(&canonical) {
            info.mtime = mtime;
            info.content = new_content;
            info.offset = None; // Full file after write
            info.limit = None;
        } else {
            self.files.insert(
                canonical,
                ReadFileInfo {
                    mtime,
                    content: new_content,
                    offset: None,
                    limit: None,
                },
            );
        }
    }

    /// Get number of tracked files.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if no files are tracked.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Iterate over all tracked files.
    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &ReadFileInfo)> {
        self.files.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_record_and_validate_read() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read(&file_path, "hello world".to_string(), None, None);

        // Should pass validation
        assert!(state.validate_for_edit(&file_path, true).is_ok());
        assert!(state.was_file_read(&file_path));
    }

    #[test]
    fn test_not_read_error() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let state = ReadFileState::new();

        // Should fail - file not read
        let result = state.validate_for_edit(&file_path, true);
        assert!(matches!(result, Err(FileValidationError::NotRead { .. })));
    }

    #[test]
    fn test_modified_since_read_error() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read(&file_path, "hello world".to_string(), None, None);

        // Modify the file externally
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&file_path, "modified content").unwrap();

        // Should fail - mtime changed
        let result = state.validate_for_edit(&file_path, true);
        assert!(matches!(
            result,
            Err(FileValidationError::ModifiedSinceRead { .. })
        ));
    }

    #[test]
    fn test_update_after_write() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read(&file_path, "hello world".to_string(), None, None);

        // Simulate a write
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&file_path, "new content").unwrap();
        state.update_after_write(&file_path, "new content".to_string());

        // Should pass validation (mtime was updated)
        assert!(state.validate_for_edit(&file_path, true).is_ok());
    }

    #[test]
    fn test_skip_mtime_check() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read(&file_path, "hello world".to_string(), None, None);

        // Modify the file externally
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&file_path, "modified content").unwrap();

        // Should pass if we skip mtime check
        assert!(state.validate_for_edit(&file_path, false).is_ok());
    }

    #[test]
    fn test_get_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read(&file_path, "hello world".to_string(), None, None);

        assert_eq!(state.get_content(&file_path), Some("hello world"));
    }

    #[test]
    fn test_len_and_is_empty() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        assert!(state.is_empty());
        assert_eq!(state.len(), 0);

        state.record_read(&file_path, "hello world".to_string(), None, None);
        assert!(!state.is_empty());
        assert_eq!(state.len(), 1);
    }
}
