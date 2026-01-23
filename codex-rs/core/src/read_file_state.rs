//! Session-level file read state tracking.
//!
//! Tracks file paths that have been read during the session for compaction recovery.
//! When compaction occurs, recently read files can be restored to provide context.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

/// Session-level tracking of files that have been read.
///
/// Used for compaction recovery - tracks which files were read so their
/// content can be restored after context compaction.
#[derive(Debug, Default)]
pub struct ReadFileState {
    /// Map of canonicalized file path to when it was recorded.
    files: HashMap<PathBuf, Instant>,
}

impl ReadFileState {
    /// Create a new empty ReadFileState.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a file has been read.
    ///
    /// # Arguments
    /// * `path` - The file path (will be canonicalized)
    pub fn record_read_path(&mut self, path: &Path) {
        let canonical = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.files.insert(canonical, Instant::now());
    }

    /// Get the most recently read file paths for compaction restoration.
    /// Sorted by most recent first.
    pub fn get_recent_files(&self, limit: usize) -> Vec<&PathBuf> {
        let mut files: Vec<_> = self.files.iter().collect();
        files.sort_by(|a, b| b.1.cmp(a.1)); // most recent first
        files.into_iter().take(limit).map(|(p, _)| p).collect()
    }

    /// Get number of tracked files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if no files are tracked.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Clear all tracked files (called after compaction).
    pub fn clear(&mut self) {
        self.files.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_record_and_get_recent() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read_path(&file_path);

        assert_eq!(state.len(), 1);
        assert!(!state.is_empty());

        let recent = state.get_recent_files(10);
        assert_eq!(recent.len(), 1);
        assert!(recent[0].ends_with("test.txt"));
    }

    #[test]
    fn test_get_recent_files_sorted_by_recency() {
        let temp_dir = TempDir::new().unwrap();

        let file1 = temp_dir.path().join("file1.txt");
        let file2 = temp_dir.path().join("file2.txt");
        let file3 = temp_dir.path().join("file3.txt");

        fs::write(&file1, "content1").unwrap();
        fs::write(&file2, "content2").unwrap();
        fs::write(&file3, "content3").unwrap();

        let mut state = ReadFileState::new();

        // Record in order: file1, file2, file3
        state.record_read_path(&file1);
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.record_read_path(&file2);
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.record_read_path(&file3);

        // Should return most recent first: file3, file2
        let recent = state.get_recent_files(2);
        assert_eq!(recent.len(), 2);
        assert!(recent[0].ends_with("file3.txt"));
        assert!(recent[1].ends_with("file2.txt"));
    }

    #[test]
    fn test_reread_updates_recency() {
        let temp_dir = TempDir::new().unwrap();

        let file1 = temp_dir.path().join("file1.txt");
        let file2 = temp_dir.path().join("file2.txt");

        fs::write(&file1, "content1").unwrap();
        fs::write(&file2, "content2").unwrap();

        let mut state = ReadFileState::new();

        // Record file1 first, then file2
        state.record_read_path(&file1);
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.record_read_path(&file2);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Re-read file1 - should become most recent
        state.record_read_path(&file1);

        let recent = state.get_recent_files(2);
        assert_eq!(recent.len(), 2);
        assert!(recent[0].ends_with("file1.txt")); // file1 is now most recent
        assert!(recent[1].ends_with("file2.txt"));
    }

    #[test]
    fn test_clear() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello world").unwrap();

        let mut state = ReadFileState::new();
        state.record_read_path(&file_path);
        assert_eq!(state.len(), 1);

        state.clear();
        assert!(state.is_empty());
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn test_nonexistent_path_still_recorded() {
        let mut state = ReadFileState::new();
        let fake_path = PathBuf::from("/nonexistent/file.txt");

        // Should still record (using the path as-is since canonicalize fails)
        state.record_read_path(&fake_path);
        assert_eq!(state.len(), 1);
    }
}
