//! # Directory Walk Module (`dir_walk_module.rs`)
//!
//! Provides iterative (non-recursive) directory traversal functionality
//! without third-party dependencies. Designed for production use with
//! comprehensive error handling and cross-platform support.
//!
//! ## Project Context
//! This module replaces `walkdir` crate usage to eliminate third-party
//! dependencies while maintaining clarity and maintainability. Used for:
//! - Scanning team channel directories for TOML/GPGTOML files
//! - Computing directory content hashes for change detection
//! - Loading message files in sorted order
//!
//! ## Platform Support
//! - Linux (including Android/Termux)
//! - Windows
//! - macOS (including BSD variants)
//!
//! ## Safety and Error Handling
//! All errors are caught and converted to Result types. No panics occur
//! in production. Errors do not expose sensitive system information.
//! Three-tier assertion pattern is used throughout:
//! 1. Debug assertions (`#[cfg(all(debug_assertions, not(test)))]`) for
//!    development-time invariant checking
//! 2. Test assertions (`#[cfg(test)]`) in cargo test functions only
//! 3. Production catches that return `Result` or skip gracefully
//!
//! ## Design: Why Unit Variants for Errors
//! Error variants carry no String payload because:
//! 1. Production must not expose paths, contents, or system details
//! 2. Debug diagnostics are printed at the error site with
//!    `#[cfg(debug_assertions)]` before the error is returned
//! 3. Each variant name (plus its doc-comment prefix) uniquely identifies
//!    the failure location — no runtime string needed
//!
//! ## Bounds and Limits
//! Queue size and per-directory entry buffer are bounded by configurable
//! limits with sensible defaults. Depth arithmetic uses checked addition.
//! These bounds prevent unbounded memory growth from adversarial or
//! pathological directory structures.

use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/*
Symlinks:
1. slim version, no simplinks
2. file-system specific versions? windows specific? redox? posix?

*/

// ============================================================================
// CONSTANTS — Default Upper Bounds
// ============================================================================

/// Default maximum number of directories that may be enqueued simultaneously.
///
/// Prevents unbounded memory growth if traversing a directory tree with
/// millions of subdirectories. Configurable via `WalkConfig::max_queue_size`.
/// When exceeded, new subdirectories are silently skipped (not enqueued),
/// and an error is yielded if `continue_on_error` is false.
///
/// 100,000 directories × ~256 bytes per PathBuf ≈ ~25 MB worst case.
const DEFAULT_MAX_QUEUE_SIZE: usize = 100_000;

/// Default maximum entries READ FROM FILESYSTEM per single directory.
///
/// Bounds the number of entries read from any one `fs::read_dir()` call.
/// Counts filesystem I/O operations, not entries yielded to the caller.
/// See `WalkConfig::max_entries_per_dir` for full explanation.
///
/// 50,000 entries × ~300 bytes per DirEntry ≈ ~15 MB worst case.
const DEFAULT_MAX_ENTRIES_PER_DIR: usize = 50_000;

// ============================================================================
// ERROR TYPES
// ============================================================================

/// Errors that can occur during directory walking.
///
/// Variants are unit types (no payload) because:
/// - Production must not expose sensitive system information
/// - Debug diagnostics are printed at the error site before returning
/// - Variant names uniquely identify the failure category
///
/// Each variant's doc comment includes a prefix code (e.g. DWEM) that
/// matches the prefix used in debug-only eprintln! calls at error sites,
/// allowing developers to trace errors from debug output to source code.
#[derive(Debug)]
pub enum WalkError {
    /// Failed to read directory entry metadata.
    /// Debug-site prefix: DWEM (Dir Walk Entry Metadata)
    EntryMetadata,

    /// Failed to read directory contents.
    /// Debug-site prefix: DWRD (Dir Walk Read Dir)
    ReadDirectory,

    /// General I/O error during walk (from std::io::Error conversion).
    /// Debug-site prefix: DWIO (Dir Walk IO)
    IoError,

    /// Depth arithmetic overflow (directory nesting exceeds usize::MAX).
    /// Debug-site prefix: DWDO (Dir Walk Depth Overflow)
    DepthOverflow,

    /// Directory queue exceeded configured maximum size.
    /// Debug-site prefix: DWQS (Dir Walk Queue Size)
    QueueSizeExceeded,

    /// Single directory contained more entries than configured limit.
    /// Debug-site prefix: DWEL (Dir Walk Entry Limit)
    EntryLimitExceeded,
}

/// Display implementation for WalkError.
///
/// Production-safe: messages are terse, contain no paths, no file contents,
/// no environment variables, no internal implementation details.
/// Each message includes the unique prefix code for tracing to source.
impl fmt::Display for WalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalkError::EntryMetadata => write!(f, "DWEM: entry metadata read failed"),
            WalkError::ReadDirectory => write!(f, "DWRD: directory read failed"),
            WalkError::IoError => write!(f, "DWIO: io operation failed"),
            WalkError::DepthOverflow => write!(f, "DWDO: depth overflow"),
            WalkError::QueueSizeExceeded => write!(f, "DWQS: queue size limit exceeded"),
            WalkError::EntryLimitExceeded => write!(f, "DWEL: entry limit per directory exceeded"),
        }
    }
}

/// Implements std::error::Error for composability with other error types.
///
/// No source() chaining because WalkError carries no inner error payload
/// (by design — production must not expose underlying io::Error details).
impl std::error::Error for WalkError {}

impl From<io::Error> for WalkError {
    fn from(_err: io::Error) -> Self {
        #[cfg(debug_assertions)]
        eprintln!("DWIO: io::Error conversion: {}", _err);

        WalkError::IoError
    }
}

// ============================================================================
// DIRECTORY ENTRY TYPE
// ============================================================================

/// Represents a single entry encountered during directory walk.
///
/// Provides safe access to entry metadata without exposing
/// raw OS-specific types. Metadata (is_dir, is_file) is captured
/// at discovery time so the caller does not need a second stat call.
///
/// ## Design: No Derived Debug
/// `Debug` is manually implemented to avoid leaking full file paths
/// in production log output. The manual impl shows only the file name
/// (not the full path) and metadata flags.
#[derive(Clone)]
pub struct DirEntry {
    /// Full path to this entry (as resolved from the walk root).
    path: PathBuf,

    /// Depth relative to the walk root (0 = root's immediate children).
    depth: usize,

    /// Whether this entry is a directory (cached from metadata).
    is_dir: bool,

    /// Whether this entry is a regular file (cached from metadata).
    is_file: bool,
}

/// Manual Debug impl for DirEntry.
///
/// Production-safe: shows only the file name component (not the full path),
/// depth, and type flags. This prevents accidental path leakage through
/// debug formatting in logs or error messages.
///
/// In debug builds, the full path is included for developer convenience.
impl fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.file_name().unwrap_or("<non-utf8>");

        #[cfg(debug_assertions)]
        {
            f.debug_struct("DirEntry")
                .field("path", &self.path)
                .field("name", &name)
                .field("depth", &self.depth)
                .field("is_dir", &self.is_dir)
                .field("is_file", &self.is_file)
                .finish()
        }

        #[cfg(not(debug_assertions))]
        {
            f.debug_struct("DirEntry")
                .field("name", &name)
                .field("depth", &self.depth)
                .field("is_dir", &self.is_dir)
                .field("is_file", &self.is_file)
                .finish()
        }
    }
}

impl DirEntry {
    /// Get the full path to this entry.
    ///
    /// # Returns
    /// Reference to the path. Path is absolute or relative depending on
    /// what was originally provided to the walker as the root.
    ///
    /// # Security Note
    /// Callers should not include this path in user-facing error messages
    /// in production builds. Use `file_name()` for safe display.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the depth of this entry relative to the walk root.
    ///
    /// # Returns
    /// - 0 for immediate children of the root directory
    /// - 1 for grandchildren, etc.
    ///
    /// Note: The root directory itself is not yielded as an entry.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Check if this entry is a directory.
    ///
    /// # Returns
    /// `true` if directory, `false` otherwise.
    /// Note: Symlinks are resolved by `std::fs::metadata`, so a symlink
    /// pointing to a directory returns `true`.
    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    /// Check if this entry is a regular file.
    ///
    /// # Returns
    /// `true` if regular file, `false` otherwise.
    pub fn is_file(&self) -> bool {
        self.is_file
    }

    /// Get file name as a borrowed string slice (zero allocation).
    ///
    /// Borrows directly from the internal PathBuf — no heap allocation,
    /// no String creation.
    ///
    /// # Returns
    /// `Some(&str)` if the filename component exists and is valid UTF-8,
    /// `None` if the path has no filename component or contains non-UTF-8
    /// bytes.
    ///
    /// # Project Context
    /// Used for extension filtering (e.g. ".toml", ".gpgtoml"), numeric
    /// prefix sorting (e.g. "1__message.toml"), and display purposes.
    /// Callers needing an owned String can call `.to_string()` on the
    /// returned `&str`.
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name().and_then(|n| n.to_str())
    }
}

// ============================================================================
// DIRECTORY WALKER CONFIGURATION
// ============================================================================

/// Configuration for directory walk behavior.
///
/// Built using method chaining. All fields have sensible defaults.
///
/// ## Example
/// ```rust,no_run
/// # use crate::dir_walk_module::WalkConfig;
/// let config = WalkConfig::new()
///     .max_depth(2)
///     .yield_directories(false)
///     .continue_on_error(true)
///     .max_queue_size(50_000)
///     .max_entries_per_dir(10_000);
/// ```
///
/// ## Bounds
/// `max_queue_size` and `max_entries_per_dir` prevent unbounded memory
/// growth from pathological directory structures (e.g., millions of
/// subdirectories or millions of files in a single directory).
#[derive(Debug, Clone)]
pub struct WalkConfig {
    /// Maximum depth to traverse (None = unlimited).
    ///
    /// - `None`: Walk all nested directories (iterative, not recursive)
    /// - `Some(0)`: Only read the root directory's immediate entries
    /// - `Some(1)`: Read root entries and one level of subdirectories
    /// - `Some(n)`: Read up to n levels of subdirectories
    max_depth: Option<usize>,

    /// Whether to yield directory entries themselves in results.
    ///
    /// - `true`: Yield both files and directories
    /// - `false`: Only yield files (directories are still traversed
    ///   internally but not returned to the caller)
    yield_directories: bool,

    /// Whether to continue walking when an individual entry errors.
    ///
    /// - `true`: Skip entries that cause errors, continue walking
    /// - `false`: Return the error and halt iteration
    continue_on_error: bool,

    /// Maximum number of directories allowed in the traversal queue.
    ///
    /// Prevents unbounded memory growth from deeply branching trees.
    /// When exceeded, new subdirectories are not enqueued. An error
    /// is yielded if `continue_on_error` is false.
    max_queue_size: usize,

    /// Maximum number of entries READ FROM FILESYSTEM per single directory.
    ///
    /// This bounds the number of `read_dir()` iteration steps performed
    /// on any single directory. It counts every entry the OS returns,
    /// regardless of whether that entry is a file, a directory, whether
    /// it passes yield filters, or whether it is ultimately returned
    /// to the caller.
    ///
    /// ## What This Limits
    /// - Filesystem I/O operations (the `read_dir` iterator advances)
    /// - Memory consumed by enqueued subdirectories
    /// - Time spent processing a single directory
    ///
    /// ## What This Does NOT Limit
    /// - The number of entries yielded to the caller (use `.take(N)`
    ///   on the iterator for that)
    /// - The total number of entries across all directories
    /// - The number of directories in the traversal queue (use
    ///   `max_queue_size` for that)
    ///
    /// ## Why It Counts I/O Operations, Not Yielded Entries
    /// If the counter only incremented when an entry was yielded,
    /// then setting `yield_directories(false)` on a directory
    /// containing 1,000,000 subdirectories and 0 files would cause
    /// the loop to read all 1,000,000 entries (counter never
    /// increments because nothing is yielded). That defeats the
    /// purpose of the limit entirely.
    ///
    /// ## Example
    /// ```text
    /// Directory contains: 50 subdirectories, 50 files (100 total)
    /// Config: max_entries_per_dir = 20, yield_directories = false
    ///
    /// Result: Reads 20 entries from the OS (mix of dirs and files),
    ///         enqueues whichever subdirectories appear in those 20,
    ///         yields only the files among those 20 to the caller.
    ///         The remaining 80 entries are never read.
    /// ```
    ///
    /// Default: 50,000
    /// 50,000 entries × ~300 bytes per DirEntry ≈ ~15 MB worst case.
    max_entries_per_dir: usize,
}

impl Default for WalkConfig {
    fn default() -> Self {
        WalkConfig {
            max_depth: None,
            yield_directories: true,
            continue_on_error: true,
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            max_entries_per_dir: DEFAULT_MAX_ENTRIES_PER_DIR,
        }
    }
}

impl WalkConfig {
    /// Create new config with default settings.
    ///
    /// Defaults:
    /// - Unlimited depth
    /// - Yield directories: true
    /// - Continue on error: true
    /// - Max queue size: 100,000
    /// - Max entries per dir: 50,000
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum traversal depth.
    ///
    /// # Arguments
    /// * `depth` - Maximum depth (0 = root entries only, 1 = one level of
    ///   subdirectories, etc.)
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Set whether to yield directory entries in results.
    ///
    /// # Arguments
    /// * `yield_dirs` - If `true`, directories appear in iteration results.
    ///   Subdirectories are always traversed regardless of this setting.
    pub fn yield_directories(mut self, yield_dirs: bool) -> Self {
        self.yield_directories = yield_dirs;
        self
    }

    /// Set error handling behavior.
    ///
    /// # Arguments
    /// * `skip_errors` - If `true`, skip errored entries and continue.
    ///   If `false`, return the error and stop iteration.
    pub fn continue_on_error(mut self, skip_errors: bool) -> Self {
        self.continue_on_error = skip_errors;
        self
    }

    /// Set maximum directory queue size.
    ///
    /// # Arguments
    /// * `size` - Maximum number of directories allowed in the traversal
    ///   queue simultaneously. Zero is treated as "no directories enqueued"
    ///   which effectively means only the root is read.
    pub fn max_queue_size(mut self, size: usize) -> Self {
        self.max_queue_size = size;
        self
    }

    /// Set maximum entries buffered per single directory read.
    ///
    /// # Arguments
    /// * `limit` - Maximum entries to buffer from one directory. Zero
    ///   means no entries are read (effectively skips all content).
    pub fn max_entries_per_dir(mut self, limit: usize) -> Self {
        self.max_entries_per_dir = limit;
        self
    }
}

// ============================================================================
// DIRECTORY WALKER (ITERATIVE, NON-RECURSIVE)
// ============================================================================

/// Iterative directory walker that avoids recursion.
///
/// ## Design
/// Uses a VecDeque as a work queue (breadth-first). Each directory is
/// read once, its entries buffered, and subdirectories enqueued for
/// later processing. This prevents stack overflow on deeply nested
/// directories and avoids recursion entirely.
///
/// ## Bounds
/// - Queue size is bounded by `WalkConfig::max_queue_size`
/// - Per-directory entry buffer is bounded by `WalkConfig::max_entries_per_dir`
/// - Depth arithmetic uses checked addition to prevent overflow
///
/// ## Iteration Model
/// Implements `Iterator<Item = Result<DirEntry, WalkError>>` so it
/// can be used directly in `for` loops, `.filter_map()`, `.collect()`,
/// and other standard iterator combinators.
pub struct DirWalker {
    /// Queue of (directory_path, depth) pairs still to be read.
    /// Directories are read in FIFO order (breadth-first).
    /// Bounded by `config.max_queue_size`.
    queue: VecDeque<(PathBuf, usize)>,

    /// Walk behavior configuration (immutable after construction).
    config: WalkConfig,

    /// Buffer of entries from the most recently read directory.
    /// Entries are yielded one at a time via `next()`.
    /// Bounded by `config.max_entries_per_dir`.
    current_entries: VecDeque<DirEntry>,

    /// Set to `true` when a fatal error occurs (continue_on_error=false).
    /// Once set, `next()` always returns `None`.
    fatal_error: bool,
}

impl DirWalker {
    /// Create a new directory walker starting at the given path.
    ///
    /// The root directory itself is enqueued for reading; its children
    /// become the first entries yielded.
    ///
    /// # Arguments
    /// * `root` - Starting directory path
    /// * `config` - Walk configuration (depth limits, bounds, etc.)
    ///
    /// # Production Behavior
    /// If `root` does not exist or is not a directory, the walker will
    /// yield an error on first iteration (or yield nothing if
    /// `continue_on_error` is true). No panic occurs.
    pub fn new(root: &Path, config: WalkConfig) -> Self {
        // =================================================
        // Debug-Assert, Test-Assert, Production-Catch-Handle
        // =================================================

        // Debug-only: warn if queue bounds are zero (likely misconfiguration)
        #[cfg(all(debug_assertions, not(test)))]
        {
            if config.max_queue_size == 0 {
                eprintln!("DW_DBG: max_queue_size is 0 — no subdirectories will be traversed");
            }
            if config.max_entries_per_dir == 0 {
                eprintln!("DW_DBG: max_entries_per_dir is 0 — no entries will be yielded");
            }
        }

        let mut queue = VecDeque::new();
        // Enqueue root at depth 0 — its children will be yielded at depth 0
        queue.push_back((root.to_path_buf(), 0));

        DirWalker {
            queue,
            config,
            current_entries: VecDeque::new(),
            fatal_error: false,
        }
    }

    /// Create a walker with default configuration (unlimited depth,
    /// yield all entries, continue on error, default bounds).
    ///
    /// # Arguments
    /// * `root` - Starting directory path
    pub fn from_path(root: &Path) -> Self {
        Self::new(root, WalkConfig::default())
    }

    /// Read one directory from the filesystem and populate `current_entries`.
    ///
    /// This is the core I/O function. It reads entries from a single
    /// directory, enqueues discovered subdirectories for later traversal,
    /// and buffers entries that pass yield filters into `current_entries`
    /// for the iterator to return.
    ///
    /// Called by `next()` when `current_entries` is empty and the queue
    /// still contains directories to process.
    ///
    /// # Arguments
    /// * `dir_path` - Path to the directory to read
    /// * `depth` - Depth of entries found in this directory (0 = root's
    ///   immediate children)
    ///
    /// # Returns
    /// * `Ok(())` - Directory was read (entries may or may not have been
    ///   buffered, depending on content and filters)
    /// * `Err(WalkError)` - A fatal error occurred (only when
    ///   `continue_on_error` is false)
    ///
    /// # How Entry Counting Works
    /// The counter `entries_read_this_dir` increments for every entry
    /// successfully read from the filesystem, BEFORE any yield filtering.
    /// This means the limit bounds actual I/O operations performed on
    /// this directory, not the number of entries returned to the caller.
    ///
    /// ```text
    /// for each entry from OS:
    ///     if entries_read_this_dir >= limit → stop reading
    ///     read entry from filesystem
    ///     read metadata from filesystem
    ///     entries_read_this_dir += 1          ← counts I/O, not yield
    ///     if directory → enqueue for later
    ///     if passes yield filter → buffer for caller
    /// ```
    ///
    /// This design prevents a directory with millions of non-yielded
    /// entries (e.g. subdirectories when `yield_directories=false`)
    /// from consuming unbounded I/O and memory.
    ///
    /// # Bounds Enforced (in order)
    /// 1. Depth limit: checked before any I/O; returns `Ok(())` if
    ///    beyond `max_depth`
    /// 2. Entry limit: checked per entry during reading; stops the
    ///    `read_dir` loop when reached
    /// 3. Queue size: checked before enqueuing each subdirectory;
    ///    skips enqueue if queue is full
    /// 4. Depth overflow: checked via `checked_add` before computing
    ///    next depth level
    fn read_directory(&mut self, dir_path: &Path, depth: usize) -> Result<(), WalkError> {
        // =================================================
        // Debug-Assert, Test-Assert, Production-Catch-Handle
        // =================================================

        // Debug-only: verify current_entries was drained before refill
        #[cfg(all(debug_assertions, not(test)))]
        {
            if !self.current_entries.is_empty() {
                eprintln!(
                    "DW_DBG: read_directory called with {} buffered entries still pending",
                    self.current_entries.len()
                );
            }
        }

        // Production catch: check depth limit before doing any I/O
        if let Some(max_depth) = self.config.max_depth {
            if depth > max_depth {
                return Ok(()); // Beyond limit, silently skip
            }
        }

        // Attempt to read directory
        let read_dir = match fs::read_dir(dir_path) {
            Ok(rd) => rd,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("DWRD: Failed to read directory at depth {}: {}", depth, _e);
                return Err(WalkError::ReadDirectory);
            }
        };

        // Track entries READ from this single directory (not just yielded).
        // This bounds the I/O and queue growth from any single directory,
        // regardless of whether entries are yielded to the caller.
        let mut entries_read_this_dir: usize = 0;

        // Process each entry in directory
        for entry_result in read_dir {
            // Production catch: enforce per-directory entry limit.
            // This check happens BEFORE processing each entry, bounding
            // total work done per directory regardless of yield settings.
            if entries_read_this_dir >= self.config.max_entries_per_dir {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DWEL: Entry limit ({}) reached; read {} entries from directory at depth {}",
                    self.config.max_entries_per_dir, entries_read_this_dir, depth
                );

                if self.config.continue_on_error {
                    break; // Stop reading this directory, continue walk
                } else {
                    return Err(WalkError::EntryLimitExceeded);
                }
            }

            // Get directory entry
            let entry = match entry_result {
                Ok(e) => e,
                Err(_e) => {
                    #[cfg(debug_assertions)]
                    eprintln!("DWRD: Failed to read dir entry at depth {}: {}", depth, _e);

                    if self.config.continue_on_error {
                        continue;
                    } else {
                        return Err(WalkError::ReadDirectory);
                    }
                }
            };

            let entry_path = entry.path();

            // Get metadata to determine file type
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_e) => {
                    #[cfg(debug_assertions)]
                    eprintln!("DWEM: Failed to get metadata at depth {}: {}", depth, _e);

                    if self.config.continue_on_error {
                        continue;
                    } else {
                        return Err(WalkError::EntryMetadata);
                    }
                }
            };

            // ────────────────────────────────────────────────────
            // COUNT I/O OPERATIONS, NOT YIELDED ENTRIES
            // ────────────────────────────────────────────────────
            // This increment MUST be here — after successful metadata
            // read, BEFORE the yield decision. If this were inside
            // the `if should_yield` block, then entries filtered out
            // by yield settings (e.g. directories when
            // `yield_directories=false`) would not be counted, and
            // the limit would fail to bound I/O on directories
            // containing only non-yielded entry types.
            // ────────────────────────────────────────────────────
            entries_read_this_dir += 1;

            // Skip symlinks entirely
            if metadata.is_symlink() {
                continue;
            }

            let is_dir = metadata.is_dir();
            let is_file = metadata.is_file();

            // Enqueue subdirectories for later processing (with bounds checks)
            if is_dir {
                // Checked depth arithmetic: prevent usize overflow
                let next_depth = match depth.checked_add(1) {
                    Some(d) => d,
                    None => {
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "DWDO: Depth overflow at depth {} — skipping subdirectory",
                            depth
                        );

                        if self.config.continue_on_error {
                            continue;
                        } else {
                            return Err(WalkError::DepthOverflow);
                        }
                    }
                };

                let should_descend = match self.config.max_depth {
                    None => true,
                    Some(max_depth) => next_depth <= max_depth,
                };

                if should_descend {
                    // Production catch: enforce queue size limit
                    if self.queue.len() >= self.config.max_queue_size {
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "DWQS: Queue size limit ({}) reached — skipping subdirectory at depth {}",
                            self.config.max_queue_size, next_depth
                        );

                        if !self.config.continue_on_error {
                            return Err(WalkError::QueueSizeExceeded);
                        }
                        // If continue_on_error, just skip enqueuing this subdir
                    } else {
                        self.queue.push_back((entry_path.clone(), next_depth));
                    }
                }
            }

            // Decide whether to yield this entry to the caller
            let should_yield = if is_dir {
                self.config.yield_directories
            } else {
                true
            };

            if should_yield {
                self.current_entries.push_back(DirEntry {
                    path: entry_path,
                    depth,
                    is_dir,
                    is_file,
                });
            }
        }

        Ok(())
    }
}

// ============================================================================
// ITERATOR IMPLEMENTATION
// ============================================================================

impl Iterator for DirWalker {
    type Item = Result<DirEntry, WalkError>;

    /// Yield the next entry in the directory walk.
    ///
    /// # Returns
    /// - `Some(Ok(entry))` — next file or directory found
    /// - `Some(Err(e))` — error occurred (only when continue_on_error=false)
    /// - `None` — walk complete (or halted after fatal error)
    ///
    /// # Algorithm
    /// 1. If `fatal_error` is set, return `None` immediately.
    /// 2. If `current_entries` has buffered entries, pop and return one.
    /// 3. Otherwise, dequeue the next directory from `queue`, read it
    ///    (populating `current_entries`), and return the first entry.
    /// 4. Repeat step 3 until entries are found or queue is exhausted.
    fn next(&mut self) -> Option<Self::Item> {
        // Fatal error halts all future iteration
        if self.fatal_error {
            return None;
        }

        // Return buffered entry if available
        if let Some(entry) = self.current_entries.pop_front() {
            return Some(Ok(entry));
        }

        // Read directories from queue until we find entries or exhaust queue
        //
        // Bounded loop: queue has a finite max size (config.max_queue_size)
        // and each iteration removes one element, so this terminates.
        while let Some((dir_path, depth)) = self.queue.pop_front() {
            match self.read_directory(&dir_path, depth) {
                Ok(()) => {
                    if let Some(entry) = self.current_entries.pop_front() {
                        return Some(Ok(entry));
                    }
                    // Empty directory or all entries skipped, try next in queue
                }
                Err(e) => {
                    if self.config.continue_on_error {
                        // Skip this directory, try next
                        continue;
                    } else {
                        self.fatal_error = true;
                        return Some(Err(e));
                    }
                }
            }
        }

        // Queue exhausted, no more entries
        None
    }
}

// ============================================================================
// CONVENIENCE FUNCTIONS
// ============================================================================

/// Walk directory with default settings (unlimited depth, all entries,
/// continue on error, default bounds).
///
/// ## Project Context
/// Used as the primary entry point for directory traversal throughout
/// the project. Equivalent to the old `WalkDir::new(path)` call.
///
/// # Arguments
/// * `path` - Starting directory path
///
/// # Returns
/// Iterator over `Result<DirEntry, WalkError>`. Each `Ok(entry)` is a
/// file or directory found during traversal. Errors are skipped by default.
///
/// # Example
/// ```rust,no_run
/// # use crate::dir_walk_module::walk_dir;
/// # use std::path::Path;
/// for entry_result in walk_dir(Path::new("/some/path")) {
///     let entry = match entry_result {
///         Ok(e) => e,
///         Err(_) => continue,
///     };
///     // process entry
/// }
/// ```
pub fn walk_dir(path: &Path) -> DirWalker {
    DirWalker::from_path(path)
}

/// Walk directory with maximum depth limit.
///
/// ## Project Context
/// Used for shallow scans where only immediate children (depth 0) or
/// one level of subdirectories (depth 1) are needed. Common in:
/// - Directory content hashing (depth 1)
/// - Message file loading from a single channel directory (depth 0)
///
/// # Arguments
/// * `path` - Starting directory path
/// * `max_depth` - Maximum depth (0 = root entries only,
///   1 = root entries + one level of subdirectory entries)
///
/// # Returns
/// Iterator over `Result<DirEntry, WalkError>`.
///
/// # Example
/// ```rust,no_run
/// # use crate::dir_walk_module::walk_dir_max_depth;
/// # use std::path::Path;
/// for entry_result in walk_dir_max_depth(Path::new("/some/path"), 1) {
///     let entry = match entry_result {
///         Ok(e) => e,
///         Err(_) => continue,
///     };
///     // process entry — guaranteed depth <= 1
/// }
/// ```
pub fn walk_dir_max_depth(path: &Path, max_depth: usize) -> DirWalker {
    DirWalker::new(path, WalkConfig::new().max_depth(max_depth))
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::Instant;

    /// Helper: create a small test directory tree under `base`.
    ///
    /// Structure:
    /// ```text
    /// base/
    /// ├── file1.txt
    /// ├── dir1/
    /// │   ├── file2.txt
    /// │   └── subdir1/
    /// │       └── file3.txt
    /// └── dir2/
    ///     └── file4.txt
    /// ```
    ///
    /// # Returns
    /// `Ok(())` on success, `io::Error` on failure.
    fn create_test_tree(base: &Path) -> io::Result<()> {
        fs::create_dir_all(base.join("dir1").join("subdir1"))?;
        fs::create_dir_all(base.join("dir2"))?;

        File::create(base.join("file1.txt"))?.write_all(b"test")?;
        File::create(base.join("dir1").join("file2.txt"))?.write_all(b"test")?;
        File::create(base.join("dir1").join("subdir1").join("file3.txt"))?.write_all(b"test")?;
        File::create(base.join("dir2").join("file4.txt"))?.write_all(b"test")?;

        Ok(())
    }

    /// Helper: create a unique test directory path using a descriptive name.
    ///
    /// Uses `std::env::temp_dir()` with a unique prefix to avoid collisions
    /// between concurrent test runs. The caller is responsible for cleanup.
    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dwm_test_{}", name))
    }

    /// Helper: safely remove test directory, ignoring errors.
    fn cleanup(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }

    // ========================================================================
    // Basic Walk Tests
    // ========================================================================

    /// Test: walk_dir finds all 4 files in the test tree.
    ///
    /// Validates that unlimited-depth walk discovers files at all
    /// nesting levels (depth 0, 1, and 2).
    #[test]
    fn test_walk_finds_all_files() {
        let dir = test_dir("walk_finds_all_files");
        cleanup(&dir);

        assert!(
            create_test_tree(&dir).is_ok(),
            "test_walk_finds_all_files: failed to create test tree"
        );

        let file_count = walk_dir(&dir)
            .filter_map(|r| r.ok())
            .filter(|e| e.is_file())
            .count();

        assert_eq!(
            file_count, 4,
            "test_walk_finds_all_files: expected 4 files, got {}",
            file_count
        );

        cleanup(&dir);
    }

    /// Test: walk_dir finds directories when yield_directories is true (default).
    ///
    /// The test tree has 3 directories (dir1, dir2, subdir1).
    #[test]
    fn test_walk_finds_directories() {
        let dir = test_dir("walk_finds_dirs");
        cleanup(&dir);

        assert!(
            create_test_tree(&dir).is_ok(),
            "test_walk_finds_directories: failed to create test tree"
        );

        let dir_count = walk_dir(&dir)
            .filter_map(|r| r.ok())
            .filter(|e| e.is_dir())
            .count();

        assert_eq!(
            dir_count, 3,
            "test_walk_finds_directories: expected 3 directories, got {}",
            dir_count
        );

        cleanup(&dir);
    }

    // ========================================================================
    // Depth Limit Tests
    // ========================================================================

    /// Test: max_depth(0) returns only immediate children of root.
    ///
    /// Root has: file1.txt, dir1/, dir2/ → 3 entries at depth 0.
    /// Nothing deeper should appear.
    #[test]
    fn test_walk_max_depth_0_immediate_children_only() {
        let dir = test_dir("walk_depth_0");
        cleanup(&dir);

        assert!(
            create_test_tree(&dir).is_ok(),
            "test_walk_max_depth_0: failed to create test tree"
        );

        let entries: Vec<_> = walk_dir_max_depth(&dir, 0).filter_map(|r| r.ok()).collect();

        // All entries must be at depth 0
        for entry in &entries {
            assert_eq!(
                entry.depth(),
                0,
                "test_walk_max_depth_0: entry at depth {}, expected 0",
                entry.depth()
            );
        }

        // Should have 3 immediate children: file1.txt, dir1, dir2
        assert_eq!(
            entries.len(),
            3,
            "test_walk_max_depth_0: expected 3 entries, got {}",
            entries.len()
        );

        cleanup(&dir);
    }

    /// Test: max_depth(1) excludes file3.txt which is at depth 2.
    ///
    /// Validates that the depth boundary is correctly enforced:
    /// depth 0 and depth 1 entries are included, depth 2 is excluded.
    #[test]
    fn test_walk_max_depth_1_excludes_nested() {
        let dir = test_dir("walk_depth_1");
        cleanup(&dir);

        assert!(
            create_test_tree(&dir).is_ok(),
            "test_walk_max_depth_1: failed to create test tree"
        );

        let entries: Vec<_> = walk_dir_max_depth(&dir, 1).filter_map(|r| r.ok()).collect();

        for entry in &entries {
            assert!(
                entry.depth() <= 1,
                "test_walk_max_depth_1: depth {} exceeds limit 1",
                entry.depth()
            );
        }

        // file3.txt is at depth 2, should NOT appear
        let has_deep_file = entries.iter().any(|e| e.file_name() == Some("file3.txt"));
        assert!(
            !has_deep_file,
            "test_walk_max_depth_1: file3.txt at depth 2 should be excluded"
        );

        cleanup(&dir);
    }

    // ========================================================================
    // Configuration Tests
    // ========================================================================

    /// Test: yield_directories(false) suppresses directory entries.
    ///
    /// All yielded entries must be files. Directories are still traversed
    /// internally (files at all depths should still appear).
    #[test]
    fn test_walk_files_only_skips_directories() {
        let dir = test_dir("walk_files_only");
        cleanup(&dir);

        assert!(
            create_test_tree(&dir).is_ok(),
            "test_walk_files_only: failed to create test tree"
        );

        let walker = DirWalker::new(&dir, WalkConfig::new().yield_directories(false));

        let entries: Vec<_> = walker.filter_map(|r| r.ok()).collect();

        for entry in &entries {
            assert!(
                entry.is_file(),
                "test_walk_files_only: yielded non-file entry"
            );
        }

        // Should still find all 4 files (directories traversed, not yielded)
        assert_eq!(
            entries.len(),
            4,
            "test_walk_files_only: expected 4 files, got {}",
            entries.len()
        );

        cleanup(&dir);
    }

    /// Test: WalkConfig builder methods correctly set fields.
    ///
    /// Verifies that method chaining produces the expected configuration
    /// without relying on Debug output (checks behavior, not internals).
    #[test]
    fn test_walk_config_builder() {
        let config = WalkConfig::new()
            .max_depth(5)
            .yield_directories(false)
            .continue_on_error(false)
            .max_queue_size(42)
            .max_entries_per_dir(99);

        assert_eq!(
            config.max_depth,
            Some(5),
            "test_walk_config_builder: max_depth mismatch"
        );
        assert!(
            !config.yield_directories,
            "test_walk_config_builder: yield_directories should be false"
        );
        assert!(
            !config.continue_on_error,
            "test_walk_config_builder: continue_on_error should be false"
        );
        assert_eq!(
            config.max_queue_size, 42,
            "test_walk_config_builder: max_queue_size mismatch"
        );
        assert_eq!(
            config.max_entries_per_dir, 99,
            "test_walk_config_builder: max_entries_per_dir mismatch"
        );
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    /// Test: walking a nonexistent directory with continue_on_error=true
    /// yields zero entries (no panic, no crash).
    ///
    /// Validates production-safe behavior: errors are silently skipped,
    /// iteration ends cleanly.
    #[test]
    fn test_walk_nonexistent_directory_continues() {
        let fake_path = test_dir("nonexistent_continues");
        cleanup(&fake_path); // Ensure it does not exist

        let count = walk_dir(&fake_path).filter_map(|r| r.ok()).count();

        assert_eq!(
            count, 0,
            "test_walk_nonexistent_continues: expected 0 entries, got {}",
            count
        );
    }

    /// Test: walking a nonexistent directory with continue_on_error=false
    /// yields exactly one error then stops.
    ///
    /// Validates that fatal error mode works: one Err is returned, then
    /// iteration halts (returns None).
    #[test]
    fn test_walk_nonexistent_directory_errors() {
        let fake_path = test_dir("nonexistent_errors");
        cleanup(&fake_path);

        let walker = DirWalker::new(&fake_path, WalkConfig::new().continue_on_error(false));

        let results: Vec<_> = walker.collect();

        assert_eq!(
            results.len(),
            1,
            "test_walk_nonexistent_errors: expected 1 result, got {}",
            results.len()
        );
        assert!(
            results[0].is_err(),
            "test_walk_nonexistent_errors: expected Err, got Ok"
        );
    }

    /// Test: fatal_error flag prevents further iteration.
    ///
    /// After a fatal error, calling next() must always return None.
    #[test]
    fn test_fatal_error_halts_iteration() {
        let fake_path = test_dir("fatal_halts");
        cleanup(&fake_path);

        let mut walker = DirWalker::new(&fake_path, WalkConfig::new().continue_on_error(false));

        // First call: should be Err
        let first = walker.next();
        assert!(
            first.is_some(),
            "test_fatal_error_halts: first next() should return Some"
        );
        assert!(
            first.as_ref().map(|r| r.is_err()).unwrap_or(false),
            "test_fatal_error_halts: first result should be Err"
        );

        // Second call: should be None (iteration halted)
        let second = walker.next();
        assert!(
            second.is_none(),
            "test_fatal_error_halts: second next() should return None after fatal error"
        );

        // Third call: still None
        let third = walker.next();
        assert!(
            third.is_none(),
            "test_fatal_error_halts: third next() should still return None"
        );
    }

    // ========================================================================
    // Empty Directory Test
    // ========================================================================

    /// Test: walking an empty directory yields zero entries.
    ///
    /// Edge case: directory exists but contains nothing.
    #[test]
    fn test_walk_empty_directory() {
        let dir = test_dir("walk_empty");
        cleanup(&dir);

        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "test_walk_empty: failed to create empty directory"
        );

        let count = walk_dir(&dir).count();

        assert_eq!(
            count, 0,
            "test_walk_empty: expected 0 entries in empty dir, got {}",
            count
        );

        cleanup(&dir);
    }

    // ========================================================================
    // Bounds Enforcement Tests
    // ========================================================================

    /// Test: max_entries_per_dir limits entries from a single directory.
    ///
    /// Creates a directory with 10 files, sets limit to 3. Should yield
    /// at most 3 entries.
    #[test]
    fn test_max_entries_per_dir_limit() {
        let dir = test_dir("entry_limit");
        cleanup(&dir);

        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "test_max_entries_per_dir: failed to create directory"
        );

        // Create 10 files
        for i in 0..10 {
            let file_path = dir.join(format!("file_{}.txt", i));
            if let Ok(mut f) = File::create(&file_path) {
                let _ = f.write_all(b"data");
            }
        }

        let config = WalkConfig::new()
            .max_entries_per_dir(3)
            .continue_on_error(true);

        let walker = DirWalker::new(&dir, config);
        let count = walker.filter_map(|r| r.ok()).count();

        assert!(
            count <= 3,
            "test_max_entries_per_dir: expected at most 3 entries, got {}",
            count
        );

        cleanup(&dir);
    }

    /// Test: max_queue_size limits subdirectory enqueueing.
    ///
    /// Creates a tree with many subdirectories, sets queue limit to 1.
    /// Walk should still complete without panic, but may skip some
    /// subdirectories.
    #[test]
    fn test_max_queue_size_limit() {
        let dir = test_dir("queue_limit");
        cleanup(&dir);

        // Create 5 subdirectories each with one file
        for i in 0..5 {
            let subdir = dir.join(format!("sub_{}", i));
            if let Err(_e) = fs::create_dir_all(&subdir) {
                #[cfg(debug_assertions)]
                eprintln!("test_max_queue_size: create_dir failed: {}", _e);
                continue;
            }
            let file_path = subdir.join("file.txt");
            if let Ok(mut f) = File::create(&file_path) {
                let _ = f.write_all(b"data");
            }
        }

        let config = WalkConfig::new().max_queue_size(1).continue_on_error(true);

        let walker = DirWalker::new(&dir, config);

        // Should complete without panic. Exact count depends on traversal
        // order and queue eviction, so we just verify it runs and returns
        // fewer entries than an unlimited walk.
        let limited_count = walker.filter_map(|r| r.ok()).count();

        let unlimited_count = walk_dir(&dir).filter_map(|r| r.ok()).count();

        assert!(
            limited_count <= unlimited_count,
            "test_max_queue_size: limited ({}) should not exceed unlimited ({})",
            limited_count,
            unlimited_count
        );

        cleanup(&dir);
    }

    // ========================================================================
    // DirEntry Method Tests
    // ========================================================================

    /// Test: file_name() returns correct borrowed string for files.
    ///
    /// Validates zero-allocation borrow from internal PathBuf.
    #[test]
    fn test_dir_entry_file_name() {
        let dir = test_dir("entry_file_name");
        cleanup(&dir);

        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "test_dir_entry_file_name: failed to create directory"
        );

        let file_path = dir.join("test_file.txt");
        if let Ok(mut f) = File::create(&file_path) {
            let _ = f.write_all(b"data");
        }

        let entries: Vec<_> = walk_dir(&dir)
            .filter_map(|r| r.ok())
            .filter(|e| e.is_file())
            .collect();

        assert_eq!(
            entries.len(),
            1,
            "test_dir_entry_file_name: expected 1 file entry"
        );

        let name = entries[0].file_name();
        assert_eq!(
            name,
            Some("test_file.txt"),
            "test_dir_entry_file_name: file name mismatch"
        );

        cleanup(&dir);
    }

    /// Test: file_name() returns None for root path without filename component.
    ///
    /// Edge case: a path like "/" has no file_name().
    #[test]
    fn test_dir_entry_file_name_none_for_root() {
        let entry = DirEntry {
            path: PathBuf::from("/"),
            depth: 0,
            is_dir: true,
            is_file: false,
        };

        // "/" may or may not have a file_name depending on platform,
        // but the method must not panic regardless.
        let _name = entry.file_name();
        // No assert on value — platform-dependent. Just verify no panic.
    }

    // ========================================================================
    // WalkError Display Tests
    // ========================================================================

    /// Test: WalkError Display produces terse, prefix-coded messages.
    ///
    /// Validates that error messages do not contain paths, file contents,
    /// or other sensitive information. Each message must start with its
    /// unique prefix code.
    #[test]
    fn test_walk_error_display_messages() {
        let errors = [
            (WalkError::EntryMetadata, "DWEM"),
            (WalkError::ReadDirectory, "DWRD"),
            (WalkError::IoError, "DWIO"),
            (WalkError::DepthOverflow, "DWDO"),
            (WalkError::QueueSizeExceeded, "DWQS"),
            (WalkError::EntryLimitExceeded, "DWEL"),
        ];

        for (error, expected_prefix) in &errors {
            let msg = format!("{}", error);
            assert!(
                msg.starts_with(expected_prefix),
                "test_walk_error_display: '{}' should start with '{}'",
                msg,
                expected_prefix
            );
            // Verify no path-like content leaked
            assert!(
                !msg.contains('/') && !msg.contains('\\'),
                "test_walk_error_display: '{}' should not contain path separators",
                msg
            );
        }
    }

    // ========================================================================
    // Depth Reporting Tests
    // ========================================================================

    /// Test: entries report correct depth values.
    ///
    /// file1.txt at depth 0, file2.txt at depth 1, file3.txt at depth 2.
    #[test]
    fn test_entry_depth_values() {
        let dir = test_dir("entry_depths");
        cleanup(&dir);

        assert!(
            create_test_tree(&dir).is_ok(),
            "test_entry_depth_values: failed to create test tree"
        );

        let entries: Vec<_> = walk_dir(&dir)
            .filter_map(|r| r.ok())
            .filter(|e| e.is_file())
            .collect();

        // Find specific files and check their depths
        for entry in &entries {
            match entry.file_name() {
                Some("file1.txt") => {
                    assert_eq!(
                        entry.depth(),
                        0,
                        "test_entry_depth_values: file1.txt should be depth 0"
                    );
                }
                Some("file2.txt") => {
                    assert_eq!(
                        entry.depth(),
                        1,
                        "test_entry_depth_values: file2.txt should be depth 1"
                    );
                }
                Some("file3.txt") => {
                    assert_eq!(
                        entry.depth(),
                        2,
                        "test_entry_depth_values: file3.txt should be depth 2"
                    );
                }
                Some("file4.txt") => {
                    assert_eq!(
                        entry.depth(),
                        1,
                        "test_entry_depth_values: file4.txt should be depth 1"
                    );
                }
                _ => {
                    // Unexpected file — fail with info
                    panic!(
                        "test_entry_depth_values: unexpected file: {:?}",
                        entry.file_name()
                    );
                }
            }
        }

        cleanup(&dir);
    }

    // ========================================================================
    // TEST 1: Does the limit bound I/O or yielding?
    // ========================================================================

    /// Test: max_entries_per_dir with yield_directories=false
    ///
    /// Setup: Directory with 10 subdirs, 10 files (20 total entries)
    /// Config: max_entries_per_dir=5, yield_directories=false
    ///
    /// Expected (if limiting I/O): Read 5 entries total, yield ≤5 files
    /// Expected (if limiting yield): Read all 20, yield ≤5 files
    #[test]
    fn test_limit_semantics_io_vs_yield() {
        let dir = test_dir("limit_semantics");
        cleanup(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create 10 directories
        for i in 0..10 {
            fs::create_dir_all(dir.join(format!("subdir_{:02}", i))).unwrap();
        }

        // Create 10 files
        for i in 0..10 {
            File::create(dir.join(format!("file_{:02}.txt", i)))
                .unwrap()
                .write_all(b"test")
                .unwrap();
        }

        let config = WalkConfig::new()
            .max_entries_per_dir(5)
            .yield_directories(false)
            .continue_on_error(true);

        let walker = DirWalker::new(&dir, config);
        let yielded_files: Vec<_> = walker.filter_map(|r| r.ok()).collect();

        println!("TEST 1 RESULTS:");
        println!("  Files yielded: {}", yielded_files.len());

        // The KEY question: did we stop at 5 I/O ops, or did we read all 20?
        //
        // Version 1 (I/O limit): Reads 5 entries. Might yield 0-5 files
        //                        depending on filesystem order
        // Version 2 (yield limit): Reads all 20 entries. Yields exactly 5 files

        if yielded_files.len() == 5 {
            println!("  INTERPRETATION: Limit bounds YIELDING (Version 2 behavior)");
        } else if yielded_files.len() < 5 {
            println!("  INTERPRETATION: Limit bounds I/O (Version 1 behavior)");
            println!("  (Fewer than 5 files yielded because we stopped reading after 5 entries)");
        }

        cleanup(&dir);
    }

    // ========================================================================
    // TEST 2: Does the limit prevent unbounded I/O?
    // ========================================================================

    /// Test: Large directory with all directories, yield_directories=false
    ///
    /// Setup: Directory with 1000 subdirectories, 0 files
    /// Config: max_entries_per_dir=10, yield_directories=false
    ///
    /// Expected (Version 1): Reads 10 entries, yields 0, fast
    /// Expected (Version 2): Reads 1000 entries, yields 0, slow
    #[test]
    fn test_limit_prevents_unbounded_io() {
        let dir = test_dir("unbounded_io_test");
        cleanup(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create 1000 subdirectories
        for i in 0..1000 {
            fs::create_dir_all(dir.join(format!("subdir_{:04}", i))).unwrap();
        }

        let config = WalkConfig::new()
            .max_entries_per_dir(10)
            .yield_directories(false)
            .continue_on_error(true);

        let start = Instant::now();
        let walker = DirWalker::new(&dir, config);
        let yielded: Vec<_> = walker.filter_map(|r| r.ok()).collect();
        let duration = start.elapsed();

        println!("TEST 2 RESULTS:");
        println!("  Entries yielded: {}", yielded.len());
        println!("  Time taken: {:?}", duration);

        // If Version 1 (I/O limit): Very fast, < 1ms
        // If Version 2 (yield limit): Slower, reads all 1000 directories

        if duration.as_millis() < 10 && yielded.is_empty() {
            println!("  INTERPRETATION: I/O was bounded (Version 1 behavior)");
        } else {
            println!("  INTERPRETATION: I/O was NOT bounded (Version 2 behavior)");
        }

        cleanup(&dir);
    }

    // ========================================================================
    // TEST 3: Queue growth with non-yielded directories
    // ========================================================================

    /// Test: Does the queue grow unbounded when directories aren't yielded?
    ///
    /// Setup: Deeply nested structure (5 levels deep)
    /// Config: max_entries_per_dir=100, yield_directories=false, max_queue_size=5
    ///
    /// Expected (Version 1): Queue fills slowly (only reading limited entries)
    /// Expected (Version 2): Queue fills faster (reads all subdirs despite not yielding)
    #[test]
    fn test_queue_growth_with_limited_entries() {
        let dir = test_dir("queue_growth");
        cleanup(&dir);

        // Create: root -> 10 subdirs -> each has 10 subdirs -> each has 1 file
        fs::create_dir_all(&dir).unwrap();
        for i in 0..10 {
            let level1 = dir.join(format!("L1_{}", i));
            fs::create_dir_all(&level1).unwrap();
            for j in 0..10 {
                let level2 = level1.join(format!("L2_{}", j));
                fs::create_dir_all(&level2).unwrap();
                File::create(level2.join("file.txt"))
                    .unwrap()
                    .write_all(b"test")
                    .unwrap();
            }
        }

        let config = WalkConfig::new()
            .max_entries_per_dir(5) // Only read 5 entries per dir
            .yield_directories(false)
            .max_queue_size(20)
            .continue_on_error(true);

        let walker = DirWalker::new(&dir, config);
        let files: Vec<_> = walker.filter_map(|r| r.ok()).collect();

        println!("TEST 3 RESULTS:");
        println!("  Files found: {}", files.len());

        // Version 1: Should find ≤25 files (5 L1 dirs × 5 L2 dirs each)
        // Version 2: Should find ≤50 files (all 10 L1 dirs × 5 yielded L2 dirs)

        if files.len() <= 30 {
            println!("  INTERPRETATION: Entry limit bounded directory enqueueing (Version 1)");
        } else {
            println!("  INTERPRETATION: Entry limit did NOT bound enqueueing (Version 2)");
        }

        cleanup(&dir);
    }

    // ========================================================================
    // TEST 4: Documentation test - what does the config claim?
    // ========================================================================

    /// Test: Verify behavior matches documentation
    #[test]
    fn test_config_documentation_matches_behavior() {
        // From the docs:
        // "Maximum number of entries buffered from a single directory read.
        //  Prevents a single directory with millions of entries from consuming
        //  unbounded memory."
        //
        // This clearly states it should limit the READ operation, not just yielding.
        // Therefore, Version 1 matches the documented intent.

        let dir = test_dir("doc_match");
        cleanup(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create a directory structure that will expose the difference
        for i in 0..100 {
            fs::create_dir_all(dir.join(format!("dir_{}", i))).unwrap();
        }

        let config = WalkConfig::new()
            .max_entries_per_dir(10)
            .yield_directories(true); // Yield them, so we can count

        let walker = DirWalker::new(&dir, config);
        let dirs: Vec<_> = walker
            .filter_map(|r| r.ok())
            .filter(|e| e.is_dir())
            .collect();

        println!("TEST 4 RESULTS:");
        println!("  Directories yielded: {}", dirs.len());

        // Per documentation: should stop reading after 10 entries
        assert!(
            dirs.len() <= 10,
            "Documentation claims limit bounds I/O, but {} dirs were yielded",
            dirs.len()
        );

        cleanup(&dir);
    }

    /// Test: Queue size limit with continue_on_error=false
    ///
    /// Verifies that when the queue limit is exceeded and continue_on_error is
    /// false, the walker either:
    /// - Returns QueueSizeExceeded error and halts, OR
    /// - Completes with fewer entries than an unlimited walk
    ///
    /// The exact behavior depends on traversal order and buffering, but the
    /// important guarantee is that the queue limit is enforced and the walk
    /// degrades gracefully (no unbounded memory growth, no panic).
    #[test]
    fn test_queue_size_exceeded_strict_mode() {
        let dir = test_dir("queue_overflow_strict");
        cleanup(&dir);

        // Create a directory with many subdirectories to exceed queue limit
        fs::create_dir_all(&dir).expect("Failed to create test directory");

        // Create 10 subdirectories, each with a file
        for i in 0..10 {
            let subdir = dir.join(format!("sub_{:02}", i));
            fs::create_dir_all(&subdir).expect("Failed to create subdirectory");
            File::create(subdir.join("file.txt"))
                .expect("Failed to create file")
                .write_all(b"data")
                .expect("Failed to write file");
        }

        // Walk with very small queue limit and strict error handling
        let config = WalkConfig::new()
            .max_queue_size(2)
            .continue_on_error(false)
            .yield_directories(false); // Only yield files for clearer counting

        let mut walker = DirWalker::new(&dir, config);

        let mut file_count = 0;
        let mut encountered_error = false;

        for result in &mut walker {
            match result {
                Ok(entry) => {
                    if entry.is_file() {
                        file_count += 1;
                    }
                }
                Err(e) => {
                    // An error occurred — could be QueueSizeExceeded or other
                    #[cfg(debug_assertions)]
                    eprintln!("TEST: Encountered error in strict mode: {}", e);

                    encountered_error = true;
                    // After error with continue_on_error=false, iterator should stop
                    break;
                }
            }
        }

        // After the loop, walker.next() should return None
        assert!(
            walker.next().is_none(),
            "Iterator should be exhausted after error or completion"
        );

        // With queue limit of 2, we cannot traverse all 10 subdirectories
        // So we expect fewer than 10 files found
        assert!(
            file_count < 10,
            "Queue limit should prevent finding all files (found {}, expected < 10)",
            file_count
        );

        // The test passes if:
        // 1. We encountered an error and halted, OR
        // 2. We found fewer files than exist (queue limit worked)
        if encountered_error {
            println!("✓ Test passed: Error encountered and iteration halted");
        } else {
            println!(
                "✓ Test passed: Queue limit enforced (found {} / 10 files)",
                file_count
            );
        }

        cleanup(&dir);
    }
}
