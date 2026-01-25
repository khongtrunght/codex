//! Helper that owns the debounce/cancellation logic for `@` file searches.
//!
//! `ChatComposer` publishes *every* change of the `@token` as
//! `AppEvent::StartFileSearch(query)`.
//! This struct receives those events and decides when to actually spawn the
//! expensive search (handled in the main `App` thread). It tries to ensure:
//!
//! - Even when the user types long text quickly, they will start seeing results
//!   after a short delay using an early version of what they typed.
//! - At most one search is in-flight at any time.
//!
//! It works as follows:
//!
//! 1. First query starts a debounce timer.
//! 2. While the timer is pending, the latest query from the user is stored.
//! 3. When the timer fires, it is cleared, and a search is done for the most
//!    recent query.
//! 4. If there is a in-flight search that is not a prefix of the latest thing
//!    the user typed, it is cancelled.

use codex_file_search as file_search;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;

const MAX_FILE_SEARCH_RESULTS: NonZeroUsize = NonZeroUsize::new(20).unwrap();
const NUM_FILE_SEARCH_THREADS: NonZeroUsize = NonZeroUsize::new(2).unwrap();

/// How long to wait after a keystroke before firing the first search when none
/// is currently running. Keeps early queries more meaningful.
const FILE_SEARCH_DEBOUNCE: Duration = Duration::from_millis(100);

const ACTIVE_SEARCH_COMPLETE_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Parsed components of an @ mention query
struct ParsedQuery {
    /// Resolved absolute path to search in
    search_directory: PathBuf,
    /// Remaining pattern for fuzzy matching
    pattern: String,
}

/// Parse an @ mention query to extract directory prefix and pattern
fn parse_file_query(query: &str, base_dir: &Path) -> Result<ParsedQuery, String> {
    let trimmed = query.trim();

    // Absolute path: starts with /
    if trimmed.starts_with('/') {
        let (dir_prefix, pattern) = split_path_pattern(trimmed);
        return Ok(ParsedQuery {
            search_directory: PathBuf::from(dir_prefix),
            pattern: pattern.to_string(),
        });
    }

    // Home directory: starts with ~
    if trimmed.starts_with('~') {
        let expanded = expand_tilde(trimmed)?;
        let (dir_prefix, pattern) = split_path_pattern(&expanded);
        return Ok(ParsedQuery {
            search_directory: PathBuf::from(dir_prefix),
            pattern: pattern.to_string(),
        });
    }

    // Relative path with ../
    if trimmed.starts_with("../") || trimmed.contains("/../") {
        let (dir_prefix, pattern) = split_path_pattern(trimmed);
        let resolved = base_dir
            .join(dir_prefix)
            .canonicalize()
            .map_err(|e| format!("Cannot resolve path: {e}"))?;
        return Ok(ParsedQuery {
            search_directory: resolved,
            pattern: pattern.to_string(),
        });
    }

    // Relative path within current directory
    let (dir_prefix, pattern) = split_path_pattern(trimmed);
    let search_dir = if dir_prefix.is_empty() {
        base_dir.to_path_buf()
    } else {
        base_dir.join(dir_prefix)
    };

    Ok(ParsedQuery {
        search_directory: search_dir,
        pattern: pattern.to_string(),
    })
}

/// Split a path into directory prefix and filename pattern
/// Example: "src/utils/file" → ("src/utils/", "file")
fn split_path_pattern(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => {
            let dir = &path[..=idx]; // Include trailing /
            let pattern = &path[idx + 1..];
            (dir, pattern)
        }
        None => ("", path),
    }
}

/// Expand ~ to home directory
fn expand_tilde(path: &str) -> Result<String, String> {
    if !path.starts_with('~') {
        return Ok(path.to_string());
    }

    let home = dirs::home_dir().ok_or_else(|| "Cannot determine home directory".to_string())?;

    if path == "~" {
        Ok(home.to_string_lossy().to_string())
    } else if let Some(stripped) = path.strip_prefix("~/") {
        Ok(home.join(stripped).to_string_lossy().to_string())
    } else {
        Err("Unsupported ~ syntax (only ~ and ~/ supported)".to_string())
    }
}

/// State machine for file-search orchestration.
pub(crate) struct FileSearchManager {
    /// Unified state guarded by one mutex.
    state: Arc<Mutex<SearchState>>,

    /// Base directory (config.cwd), queries are resolved relative to this
    base_dir: PathBuf,
    app_tx: AppEventSender,
}

struct SearchState {
    /// Latest query typed by user (updated every keystroke).
    latest_query: String,

    /// true if a search is currently scheduled.
    is_search_scheduled: bool,

    /// If there is an active search, this will be the query being searched.
    active_search: Option<ActiveSearch>,
}

struct ActiveSearch {
    query: String,
    cancellation_token: Arc<AtomicBool>,
}

impl FileSearchManager {
    pub fn new(base_dir: PathBuf, tx: AppEventSender) -> Self {
        Self {
            state: Arc::new(Mutex::new(SearchState {
                latest_query: String::new(),
                is_search_scheduled: false,
                active_search: None,
            })),
            base_dir,
            app_tx: tx,
        }
    }

    /// Call whenever the user edits the `@` token.
    pub fn on_user_query(&self, query: String) {
        {
            #[expect(clippy::unwrap_used)]
            let mut st = self.state.lock().unwrap();
            if query == st.latest_query {
                // No change, nothing to do.
                return;
            }

            // Update latest query.
            st.latest_query.clear();
            st.latest_query.push_str(&query);

            // If there is an in-flight search that is definitely obsolete,
            // cancel it now.
            if let Some(active_search) = &st.active_search
                && !query.starts_with(&active_search.query)
            {
                active_search
                    .cancellation_token
                    .store(true, Ordering::Relaxed);
                st.active_search = None;
            }

            // Schedule a search to run after debounce.
            if !st.is_search_scheduled {
                st.is_search_scheduled = true;
            } else {
                return;
            }
        }

        // If we are here, we set `st.is_search_scheduled = true` before
        // dropping the lock. This means we are the only thread that can spawn a
        // debounce timer.
        let state = self.state.clone();
        let base_dir = self.base_dir.clone();
        let tx_clone = self.app_tx.clone();
        thread::spawn(move || {
            // Always do a minimum debounce, but then poll until the
            // `active_search` is cleared.
            thread::sleep(FILE_SEARCH_DEBOUNCE);
            loop {
                #[expect(clippy::unwrap_used)]
                if state.lock().unwrap().active_search.is_none() {
                    break;
                }
                thread::sleep(ACTIVE_SEARCH_COMPLETE_POLL_INTERVAL);
            }

            // The debounce timer has expired, so start a search using the
            // latest query.
            let cancellation_token = Arc::new(AtomicBool::new(false));
            let token = cancellation_token.clone();
            let query = {
                #[expect(clippy::unwrap_used)]
                let mut st = state.lock().unwrap();
                let query = st.latest_query.clone();
                st.is_search_scheduled = false;
                st.active_search = Some(ActiveSearch {
                    query: query.clone(),
                    cancellation_token: token,
                });
                query
            };

            FileSearchManager::spawn_file_search(
                query,
                base_dir,
                tx_clone,
                cancellation_token,
                state,
            );
        });
    }

    fn spawn_file_search(
        query: String,
        base_dir: PathBuf,
        tx: AppEventSender,
        cancellation_token: Arc<AtomicBool>,
        search_state: Arc<Mutex<SearchState>>,
    ) {
        let compute_indices = true;
        std::thread::spawn(move || {
            // Parse query to extract directory and pattern
            let parsed = match parse_file_query(&query, &base_dir) {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!("Failed to parse file query '{query}': {err}");
                    // Fallback: use base_dir and full query as pattern
                    ParsedQuery {
                        search_directory: base_dir.clone(),
                        pattern: query.clone(),
                    }
                }
            };

            // Hybrid approach:
            // 1. If pattern is empty and directory exists → direct listing (fast)
            // 2. If search_directory is outside cwd → shallow search (depth=1)
            // 3. Otherwise → fuzzy search with default depth limit
            let is_outside_cwd = !parsed.search_directory.starts_with(&base_dir);

            let matches = if parsed.pattern.is_empty() && parsed.search_directory.is_dir() {
                // Direct directory listing (non-recursive, fast)
                file_search::list_directory(&parsed.search_directory, MAX_FILE_SEARCH_RESULTS)
                    .map(|res| res.matches)
                    .unwrap_or_default()
            } else {
                // For paths outside cwd, only search 1 level deep (direct children)
                // For paths inside cwd, use default depth limit
                let max_depth = if is_outside_cwd {
                    Some(1)
                } else {
                    Some(file_search::DEFAULT_MAX_DEPTH)
                };

                file_search::run_with_options(
                    &parsed.pattern,
                    MAX_FILE_SEARCH_RESULTS,
                    &parsed.search_directory,
                    Vec::new(),
                    NUM_FILE_SEARCH_THREADS,
                    cancellation_token.clone(),
                    compute_indices,
                    true,
                    max_depth,
                )
                .map(|res| res.matches)
                .unwrap_or_default()
            };

            let is_cancelled = cancellation_token.load(Ordering::Relaxed);
            if !is_cancelled {
                tx.send(AppEvent::FileSearchResult { query, matches });
            }

            // Reset the active search state. Do a pointer comparison to verify
            // that we are clearing the ActiveSearch that corresponds to the
            // cancellation token we were given.
            {
                #[expect(clippy::unwrap_used)]
                let mut st = search_state.lock().unwrap();
                if let Some(active_search) = &st.active_search
                    && Arc::ptr_eq(&active_search.cancellation_token, &cancellation_token)
                {
                    st.active_search = None;
                }
            }
        });
    }
}
