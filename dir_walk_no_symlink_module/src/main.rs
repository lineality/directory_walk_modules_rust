//! # Directory Walk Module Demo (`main.rs`)
//!
//! Demonstrates all features of the custom directory walk implementation
//! without third-party dependencies.
//!
//! ## What This Program Does
//! 1. Creates a test directory structure (left for manual inspection)
//! 2. Runs 7 demos showing different walk configurations and patterns
//! 3. Prints results with clear formatting
//! 4. Provides cleanup instructions at the end
//!
//! ## Error Handling
//! All demo functions return Result. main() logs errors and continues
//! to the next demo — never panics, never halts.
//!
//! ## Project Context
//! This is a standalone demo/validation program. It exercises the same
//! directory walk patterns used in the main project for:
//! - Team channel TOML/GPGTOML scanning
//! - Directory content hash computation
//! - Sorted message file loading

mod dir_walk_no_symlink_module;
use dir_walk_no_symlink_module::{DirWalker, WalkConfig, walk_dir, walk_dir_max_depth};

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::Path;

// ============================================================================
// ERROR TYPE
// ============================================================================

/// Demo-specific errors.
///
/// Unit variants only — no String payload — because:
/// 1. Debug diagnostics are printed at the error site
/// 2. Production must not expose system details
/// 3. The variant name identifies the failure category
#[derive(Debug)]
enum DemoError {
    /// I/O failure during demo setup or execution.
    /// Debug-site prefix: DIO (Demo IO)
    Io,

    /// Walk operation failure propagated from dir_walk_module.
    /// Debug-site prefix: DWK (Demo WalK)
    Walk,
}

impl std::fmt::Display for DemoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DemoError::Io => write!(f, "DIO: demo io operation failed"),
            DemoError::Walk => write!(f, "DWK: demo walk operation failed"),
        }
    }
}

impl std::error::Error for DemoError {}

impl From<io::Error> for DemoError {
    fn from(_err: io::Error) -> Self {
        #[cfg(debug_assertions)]
        eprintln!("DIO: io::Error: {}", _err);
        DemoError::Io
    }
}

impl From<dir_walk_no_symlink_module::WalkError> for DemoError {
    fn from(_err: dir_walk_no_symlink_module::WalkError) -> Self {
        #[cfg(debug_assertions)]
        eprintln!("DWK: WalkError: {}", _err);
        DemoError::Walk
    }
}

// ============================================================================
// TEST DIRECTORY STRUCTURE CREATION
// ============================================================================

/// Create a comprehensive test directory structure for demonstration.
///
/// ## Structure Created
/// ```text
/// demo_test_dir/
/// ├── 0.toml                        (metadata file, skipped by sorter)
/// ├── 1__first_message.toml
/// ├── 2__second_message.toml
/// ├── 3__third_message.gpgtoml
/// ├── README.txt
/// ├── team_alpha/
/// │   ├── 1__alpha_msg.toml
/// │   ├── 2__alpha_msg.toml
/// │   └── subdir/
/// │       ├── 1__nested.toml
/// │       └── deep_file.txt
/// ├── team_beta/
/// │   ├── 1__beta_msg.toml
/// │   └── archive/
/// │       └── old_message.toml
/// └── temp_files/
///     ├── cache.tmp
///     └── log.txt
/// ```
///
/// # Arguments
/// * `base_path` - Base directory where test structure will be created
///
/// # Returns
/// `Ok(())` on success, `Err(DemoError)` if any file/dir creation fails.
fn create_test_directory_structure(base_path: &Path) -> Result<(), DemoError> {
    println!("\n📁 Creating test directory structure...");

    fs::create_dir_all(base_path)?;
    println!("   Created base directory");

    // -- Root-level files --
    let root_files: &[(&str, &str)] = &[
        (
            "1__first_message.toml",
            "# First message\n[message]\ncontent = \"Hello\"",
        ),
        (
            "2__second_message.toml",
            "# Second message\n[message]\ncontent = \"World\"",
        ),
        (
            "3__third_message.gpgtoml",
            "# Encrypted message\n[message]\ncontent = \"Secret\"",
        ),
        ("0.toml", "# Metadata\n[metadata]\nversion = \"1.0\""),
        (
            "README.txt",
            "This is a test directory for directory walking demos.",
        ),
    ];

    for (filename, content) in root_files {
        let file_path = base_path.join(filename);
        let mut file = File::create(&file_path)?;
        file.write_all(content.as_bytes())?;
        println!("   Created file: {}", filename);
    }

    // -- team_alpha with nested subdir --
    let team_alpha = base_path.join("team_alpha");
    fs::create_dir_all(&team_alpha)?;
    println!("   Created directory: team_alpha/");

    let alpha_files: &[(&str, &str)] = &[
        ("1__alpha_msg.toml", "[message]\nteam = \"alpha\"\nid = 1"),
        ("2__alpha_msg.toml", "[message]\nteam = \"alpha\"\nid = 2"),
    ];

    for (filename, content) in alpha_files {
        let file_path = team_alpha.join(filename);
        let mut file = File::create(&file_path)?;
        file.write_all(content.as_bytes())?;
        println!("   Created file: team_alpha/{}", filename);
    }

    let alpha_subdir = team_alpha.join("subdir");
    fs::create_dir_all(&alpha_subdir)?;
    println!("   Created directory: team_alpha/subdir/");

    let subdir_files: &[(&str, &str)] = &[
        ("deep_file.txt", "This is deeply nested"),
        ("1__nested.toml", "[message]\nnested = true"),
    ];

    for (filename, content) in subdir_files {
        let file_path = alpha_subdir.join(filename);
        let mut file = File::create(&file_path)?;
        file.write_all(content.as_bytes())?;
        println!("   Created file: team_alpha/subdir/{}", filename);
    }

    // -- team_beta with archive subdir --
    let team_beta = base_path.join("team_beta");
    fs::create_dir_all(&team_beta)?;
    println!("   Created directory: team_beta/");

    let beta_file_path = team_beta.join("1__beta_msg.toml");
    let mut beta_file = File::create(&beta_file_path)?;
    beta_file.write_all(b"[message]\nteam = \"beta\"\nid = 1")?;
    println!("   Created file: team_beta/1__beta_msg.toml");

    let beta_archive = team_beta.join("archive");
    fs::create_dir_all(&beta_archive)?;
    println!("   Created directory: team_beta/archive/");

    let archive_path = beta_archive.join("old_message.toml");
    let mut archive_file = File::create(&archive_path)?;
    archive_file.write_all(b"[message]\narchived = true")?;
    println!("   Created file: team_beta/archive/old_message.toml");

    // -- temp_files --
    let temp_files = base_path.join("temp_files");
    fs::create_dir_all(&temp_files)?;
    println!("   Created directory: temp_files/");

    let temp_entries: &[(&str, &str)] = &[
        ("cache.tmp", "temporary cache data"),
        ("log.txt", "log entry 1\nlog entry 2\nlog entry 3"),
    ];

    for (filename, content) in temp_entries {
        let file_path = temp_files.join(filename);
        let mut file = File::create(&file_path)?;
        file.write_all(content.as_bytes())?;
        println!("   Created file: temp_files/{}", filename);
    }

    println!("✅ Test directory structure created successfully!\n");

    Ok(())
}

// ============================================================================
// DEMO FUNCTIONS
// ============================================================================

/// Demo 1: Basic recursive directory walk (all files and directories).
///
/// Equivalent to the old `WalkDir::new(path)` with default settings.
/// Shows breadth-first traversal with depth indicators.
///
/// # Project Context
/// This pattern is used for full directory scans, such as scanning
/// all team channel directories for content.
fn demo_basic_recursive_walk(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 1: Basic Recursive Walk (All Files & Directories)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: walk_dir(path)\n");

    let mut file_count: usize = 0;
    let mut dir_count: usize = 0;
    let mut error_count: usize = 0;

    for entry_result in walk_dir(path) {
        match entry_result {
            Ok(entry) => {
                let depth_indent = "  ".repeat(entry.depth());
                let type_marker = if entry.is_dir() {
                    "📁"
                } else if entry.is_file() {
                    "📄"
                } else {
                    "❓"
                };

                let display_name = entry.file_name().unwrap_or("<non-utf8>");

                println!(
                    "{}{}[depth:{}] {}",
                    depth_indent,
                    type_marker,
                    entry.depth(),
                    display_name
                );

                if entry.is_file() {
                    file_count += 1;
                } else if entry.is_dir() {
                    dir_count += 1;
                }
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D1: Walk error: {}", _e);
                error_count += 1;
            }
        }
    }

    println!("\n📊 Summary:");
    println!("   Files found: {}", file_count);
    println!("   Directories found: {}", dir_count);
    println!("   Errors encountered: {}", error_count);
    println!();

    Ok(())
}

/// Demo 2: Depth-limited walk (max_depth = 1, immediate children + one level).
///
/// Equivalent to the old `WalkDir::new(path).max_depth(1)`.
///
/// # Project Context
/// Used when only shallow directory content is needed, such as listing
/// team channels in a workspace directory (one level deep).
fn demo_max_depth_walk(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 2: Max Depth Walk (Depth Limited to 1)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: walk_dir_max_depth(path, 1)\n");

    let mut count: usize = 0;

    for entry_result in walk_dir_max_depth(path, 1) {
        match entry_result {
            Ok(entry) => {
                let type_marker = if entry.is_dir() { "📁" } else { "📄" };
                let display_name = entry.file_name().unwrap_or("<non-utf8>");

                println!("{} [depth:{}] {}", type_marker, entry.depth(), display_name);
                count += 1;
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D2: Walk error: {}", _e);
            }
        }
    }

    println!("\n📊 Total entries at depth ≤ 1: {}", count);
    println!("   (Notice: No deeply nested files like 'deep_file.txt')\n");

    Ok(())
}

/// Demo 3: Walk yielding only files (directory entries suppressed).
///
/// # Project Context
/// Used when processing only files (e.g., loading message content)
/// and directory entries themselves are not needed in the result set.
fn demo_files_only_walk(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 3: Files Only Walk (No Directory Entries)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: WalkConfig::new().yield_directories(false)\n");

    let walker = DirWalker::new(path, WalkConfig::new().yield_directories(false));

    let mut file_count: usize = 0;

    for entry_result in walker {
        match entry_result {
            Ok(entry) => {
                let depth_indent = "  ".repeat(entry.depth());
                let display_name = entry.file_name().unwrap_or("<non-utf8>");

                println!("{}📄 {}", depth_indent, display_name);
                file_count += 1;
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D3: Walk error: {}", _e);
            }
        }
    }

    println!("\n📊 Total files found: {}", file_count);
    println!("   (Notice: No directory entries, only files)\n");

    Ok(())
}

/// Demo 4: Filter for .toml and .gpgtoml extensions only.
///
/// Shows the pattern used in team channel scanning for configuration
/// and message files.
///
/// # Project Context
/// Team channels store messages as .toml files and encrypted messages
/// as .gpgtoml files. This filter pattern extracts only those files
/// from a directory tree that may also contain other file types.
fn demo_extension_filter_walk(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 4: Extension Filter (.toml and .gpgtoml only)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: walk_dir + manual extension filter in loop\n");

    let mut toml_count: usize = 0;
    let mut gpgtoml_count: usize = 0;

    for entry_result in walk_dir(path) {
        let entry = match entry_result {
            Ok(e) => e,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D4: Walk error (skipping): {}", _e);
                continue;
            }
        };

        if !entry.is_file() {
            continue;
        }

        // Extract extension safely — no panic on missing or non-UTF-8
        let extension = match entry.path().extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext == "toml" || ext == "gpgtoml" => ext,
            _ => continue, // Not a target file, skip silently
        };

        let depth_indent = "  ".repeat(entry.depth());
        let display_name = entry.file_name().unwrap_or("<non-utf8>");
        let emoji = if extension == "gpgtoml" {
            "🔒"
        } else {
            "📄"
        };

        println!("{}{} {} ({})", depth_indent, emoji, display_name, extension);

        if extension == "toml" {
            toml_count += 1;
        } else {
            gpgtoml_count += 1;
        }
    }

    println!("\n📊 Summary:");
    println!("   .toml files: {}", toml_count);
    println!("   .gpgtoml files: {}", gpgtoml_count);
    println!();

    Ok(())
}

/// Demo 5: Collect files, sort by numeric prefix, process in order.
///
/// Shows the pattern used for loading message files in sorted order.
///
/// # Project Context
/// Message files are named with a numeric prefix (e.g. "1__message.toml",
/// "2__message.toml"). Loading them in numeric order ensures correct
/// chronological display. The "0.toml" file is metadata and is skipped.
fn demo_collect_and_sort(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 5: Collect, Sort by Numeric Prefix, Process");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: collect + sort_by_key + iterate\n");

    // Collect files at depth 0 only (immediate children)
    let mut entries: Vec<_> = walk_dir_max_depth(path, 0)
        .filter_map(|r| r.ok())
        .filter(|e| e.is_file())
        .collect();

    println!("📥 Collected {} files", entries.len());

    // Sort by numeric prefix before "__"
    // Files without a parseable numeric prefix sort last (u64::MAX)
    entries.sort_by_key(|entry| {
        entry
            .file_name()
            .and_then(|name| name.split("__").next())
            .and_then(|num_str| num_str.parse::<u64>().ok())
            .unwrap_or(u64::MAX) // Unparseable names sort last
    });

    println!("🔢 Sorted by numeric prefix\n");
    println!("Sorted order:");

    for (index, entry) in entries.iter().enumerate() {
        let display_name = entry.file_name().unwrap_or("<non-utf8>");

        if display_name == "0.toml" {
            println!(
                "   [{}] ⏭️  {} (metadata, skipped)",
                index + 1,
                display_name
            );
            continue;
        }

        println!("   [{}] 📄 {}", index + 1, display_name);
    }

    println!();

    Ok(())
}

/// Demo 6: Directory content hashing for change detection.
///
/// Computes a hash from file modification times and sizes at depth 0.
/// Used in passive view mode to detect when directory contents change.
///
/// # Project Context
/// The main application polls directory content hashes to detect new
/// messages or file changes without re-reading all file contents.
/// Only metadata (modification time + size) is hashed, not file contents.
fn demo_directory_hash(path: &Path) -> Result<u64, DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 6: Directory Content Hashing (Change Detection)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: walk_dir_max_depth + hash file metadata\n");

    let mut hasher = DefaultHasher::new();
    let mut files_hashed: usize = 0;

    println!("Computing hash from file metadata...");

    for entry_result in walk_dir_max_depth(path, 1) {
        let entry = match entry_result {
            Ok(e) => e,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D6: Hash walk error (skipping): {}", _e);
                continue;
            }
        };

        if !entry.is_file() {
            continue;
        }

        let metadata = match fs::metadata(entry.path()) {
            Ok(m) => m,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D6: Metadata error (skipping): {}", _e);
                continue;
            }
        };

        if let Ok(modified) = metadata.modified() {
            modified.hash(&mut hasher);
        }
        metadata.len().hash(&mut hasher);
        files_hashed += 1;

        let display_name = entry.file_name().unwrap_or("<non-utf8>");
        println!(
            "   ✓ Hashed: {} (size: {} bytes)",
            display_name,
            metadata.len()
        );
    }

    let hash_result = hasher.finish();

    println!("\n📊 Hash Result:");
    println!("   Files processed: {}", files_hashed);
    println!("   Hash value: 0x{:016x}", hash_result);
    println!("   (If directory content changes, hash will differ)\n");

    Ok(hash_result)
}

/// Demo 7: Error handling patterns (continue vs. stop on error).
///
/// Demonstrates both production-safe (continue) and debug/test (stop)
/// error handling modes.
///
/// # Project Context
/// Production code always uses continue_on_error=true so that a single
/// unreadable file does not halt the entire directory scan. Debug/test
/// builds may use continue_on_error=false to surface issues early.
fn demo_error_handling_patterns(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 7: Error Handling Patterns");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Pattern 1: Continue on error (default, production-safe)
    println!("\nPattern 1: Continue on Error (Production Safe)");
    println!("---------------------------------------------");

    let walker = DirWalker::new(path, WalkConfig::new().continue_on_error(true));

    let mut success_count: usize = 0;
    let mut error_count: usize = 0;

    for entry_result in walker {
        match entry_result {
            Ok(_) => success_count += 1,
            Err(_e) => {
                error_count += 1;
                #[cfg(debug_assertions)]
                eprintln!("   D7: Error encountered (continuing): {}", _e);
            }
        }
    }

    println!("   ✓ Processed {} entries successfully", success_count);
    println!(
        "   ⚠ Encountered {} errors (skipped and continued)",
        error_count
    );

    // Pattern 2: Stop on first error
    println!("\nPattern 2: Stop on First Error (Test/Debug)");
    println!("--------------------------------------------");

    let walker = DirWalker::new(path, WalkConfig::new().continue_on_error(false));

    let mut processed: usize = 0;
    let mut stopped_on_error = false;

    for entry_result in walker {
        match entry_result {
            Ok(_) => processed += 1,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("   D7: Stopped on error: {}", _e);

                // Production: terse output, no path or system details
                #[cfg(not(debug_assertions))]
                println!("   ❌ Stopped on first error");

                #[cfg(debug_assertions)]
                println!("   ❌ Stopped on first error: {:?}", _e);

                stopped_on_error = true;
                break;
            }
        }
    }

    if stopped_on_error {
        println!("   Processed {} entries before stopping", processed);
    } else {
        println!(
            "   Processed all {} entries (no errors occurred)",
            processed
        );
    }

    println!();

    Ok(())
}

/// Demo 8: Bounded entry reading with `max_entries_per_dir`.
///
/// Demonstrates production use of `WalkConfig::max_entries_per_dir()` to
/// cap the number of filesystem entries read from any single directory.
/// This prevents a single directory containing millions of entries from
/// consuming unbounded I/O time and memory.
///
/// ## What This Shows
/// - A directory with 20 files is walked with a per-dir limit of 5
/// - The walker reads at most 5 entries from the OS, then stops that dir
/// - Yielded count is ≤ 5 (may be fewer if directories are among the first 5
///   entries read and `yield_directories` filters them)
///
/// ## Project Context
/// In production, team channel directories or workspace roots may contain
/// an unexpectedly large number of entries (e.g. a user accidentally drops
/// thousands of files into a channel directory). Without this limit, the
/// walker would read every entry — potentially millions — consuming memory
/// and blocking the event loop. Setting `max_entries_per_dir` to a
/// reasonable ceiling (e.g. 10,000) ensures the walker degrades gracefully:
/// it reads up to the limit, enqueues discovered subdirectories within
/// that window, yields files within that window, and moves on.
///
/// ## Bound Semantics (Critical Design Note)
/// The limit counts entries READ FROM THE FILESYSTEM, not entries yielded
/// to the caller. This is intentional:
/// - If counting only yielded entries, a directory with 1,000,000
///   subdirectories and 0 files would cause unbounded I/O when
///   `yield_directories=false` (no files to count, loop never stops).
/// - Counting I/O operations bounds actual work regardless of filters.
///
/// # Arguments
/// * `path` - Root directory to walk (should contain files for meaningful output)
///
/// # Returns
/// `Ok(())` on success, `Err(DemoError)` if setup or walk fails fatally.
fn demo_max_entries_per_dir(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 8: Bounded Entry Reading (max_entries_per_dir)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: WalkConfig::new().max_entries_per_dir(5)\n");

    // -- Setup: create a subdirectory with 20 files --
    // This isolates the demo from whatever else is in `path`.
    let demo_subdir = path.join("demo8_many_files");

    // Production-safe: do not halt if cleanup fails; stale files
    // from a prior run will simply be overwritten by File::create.
    let _ = fs::remove_dir_all(&demo_subdir);

    if let Err(_e) = fs::create_dir_all(&demo_subdir) {
        #[cfg(debug_assertions)]
        eprintln!("D8: Failed to create demo8 subdirectory: {}", _e);
        println!("   ⚠ Could not create demo subdirectory, skipping demo 8.");
        return Ok(()); // Graceful skip, not halt
    }

    // Create 20 numbered files so ordering is predictable in output
    let file_creation_target: usize = 20;
    let mut files_created: usize = 0;

    for i in 0..file_creation_target {
        let file_path = demo_subdir.join(format!("entry_{:03}.txt", i));
        match File::create(&file_path) {
            Ok(mut f) => {
                // Write minimal content — we only need the entry to exist
                if f.write_all(b"data").is_ok() {
                    files_created += 1;
                }
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D8: Failed to create file {}: {}", i, _e);
                // Continue creating remaining files — partial setup is still useful
            }
        }
    }

    println!(
        "   Setup: Created {} files in demo8_many_files/",
        files_created
    );
    println!("   Limit: max_entries_per_dir = 5\n");

    // -- Walk with entry limit --
    // max_depth(0): only read the demo subdirectory's immediate children
    // max_entries_per_dir(5): stop reading after 5 entries from the OS
    // continue_on_error(true): production-safe, skip issues
    let config = WalkConfig::new()
        .max_depth(0)
        .max_entries_per_dir(5)
        .continue_on_error(true);

    let walker = DirWalker::new(&demo_subdir, config);

    let mut yielded_count: usize = 0;

    for entry_result in walker {
        match entry_result {
            Ok(entry) => {
                let display_name = entry.file_name().unwrap_or("<non-utf8>");
                println!("   📄 {}", display_name);
                yielded_count += 1;
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D8: Walk error (continuing): {}", _e);
            }
        }
    }

    println!("\n📊 Results:");
    println!("   Files in directory: {}", files_created);
    println!("   Entries yielded:    {}", yielded_count);
    println!("   Entry limit:        5");

    if yielded_count <= 5 {
        println!(
            "   ✓ Limit correctly bounded I/O — only {} entries read",
            yielded_count
        );
    } else {
        // This should not happen if the module is working correctly.
        // Production: log and continue; do not panic.
        println!("   ⚠ More entries than limit — potential bound enforcement issue");
    }

    // -- Also demonstrate with yield_directories=false to show I/O-counting --
    // Create a mixed directory: 10 subdirs + 10 files, limit to 5
    let demo_mixed = path.join("demo8_mixed");
    let _ = fs::remove_dir_all(&demo_mixed);

    if let Err(_e) = fs::create_dir_all(&demo_mixed) {
        #[cfg(debug_assertions)]
        eprintln!("D8: Failed to create demo8_mixed subdirectory: {}", _e);
        println!("\n   ⚠ Could not create mixed demo subdirectory, skipping mixed test.");
        println!();
        return Ok(());
    }

    // Create 10 subdirectories and 10 files
    for i in 0..10 {
        let subdir_path = demo_mixed.join(format!("subdir_{:02}", i));
        let _ = fs::create_dir_all(&subdir_path);
    }
    for i in 0..10 {
        let file_path = demo_mixed.join(format!("file_{:02}.txt", i));
        if let Ok(mut f) = File::create(&file_path) {
            let _ = f.write_all(b"data");
        }
    }

    println!("\n   Mixed directory test:");
    println!("   Setup: 10 subdirs + 10 files = 20 entries total");
    println!("   Config: max_entries_per_dir=5, yield_directories=false");

    let mixed_config = WalkConfig::new()
        .max_depth(0)
        .max_entries_per_dir(5)
        .yield_directories(false)
        .continue_on_error(true);

    let mixed_walker = DirWalker::new(&demo_mixed, mixed_config);
    let mut mixed_yielded: usize = 0;

    for entry_result in mixed_walker {
        match entry_result {
            Ok(entry) => {
                let display_name = entry.file_name().unwrap_or("<non-utf8>");
                println!("   📄 {}", display_name);
                mixed_yielded += 1;
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D8: Mixed walk error (continuing): {}", _e);
            }
        }
    }

    println!("   Files yielded from mixed dir: {}", mixed_yielded);
    println!("   (Limit bounds I/O reads, not just yielded entries)");
    println!();

    Ok(())
}

/// Demo 9: Bounded queue growth with `max_queue_size`.
///
/// Demonstrates production use of `WalkConfig::max_queue_size()` to cap
/// the number of subdirectories held in the traversal queue simultaneously.
/// This prevents an adversarial or pathological directory tree (e.g.
/// millions of subdirectories) from consuming unbounded memory.
///
/// ## What This Shows
/// - A tree with 15 subdirectories is walked with a queue limit of 3
/// - The walker enqueues at most 3 directories at any time
/// - Some subdirectories are silently skipped (their files are not found)
/// - An unlimited walk of the same tree finds more entries
///
/// ## Project Context
/// In production, a compromised or misconfigured workspace might contain
/// a deeply branching directory tree (e.g. symlink loops on systems where
/// metadata follows links, or automated tooling creating thousands of
/// directories). Without `max_queue_size`, the walker's VecDeque could
/// grow without bound. Setting a ceiling (e.g. 100,000 — the default)
/// ensures memory usage stays within ~25 MB even under adversarial
/// conditions. For constrained environments (embedded, mobile), a much
/// lower limit (e.g. 1,000) may be appropriate.
///
/// ## Queue Eviction Semantics
/// When the queue is full, newly discovered subdirectories are simply
/// not enqueued. They are not yielded as errors (unless `continue_on_error`
/// is false). The walker continues processing directories already in the
/// queue. This means:
/// - Some branches of the tree may be silently unexplored
/// - The walk always terminates
/// - Memory usage is bounded
///
/// # Arguments
/// * `path` - Root directory (a subdirectory with many branches is created inside)
///
/// # Returns
/// `Ok(())` on success, `Err(DemoError)` if setup fails fatally.
fn demo_max_queue_size(path: &Path) -> Result<(), DemoError> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("DEMO 9: Bounded Queue Growth (max_queue_size)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Usage: WalkConfig::new().max_queue_size(3)\n");

    // -- Setup: create a wide tree with 15 subdirectories, each containing 1 file --
    let demo_subdir = path.join("demo9_wide_tree");

    let _ = fs::remove_dir_all(&demo_subdir);

    if let Err(_e) = fs::create_dir_all(&demo_subdir) {
        #[cfg(debug_assertions)]
        eprintln!("D9: Failed to create demo9 subdirectory: {}", _e);
        println!("   ⚠ Could not create demo subdirectory, skipping demo 9.");
        return Ok(());
    }

    let branch_count: usize = 15;
    let mut branches_created: usize = 0;

    for i in 0..branch_count {
        let branch_dir = demo_subdir.join(format!("branch_{:02}", i));
        if let Err(_e) = fs::create_dir_all(&branch_dir) {
            #[cfg(debug_assertions)]
            eprintln!("D9: Failed to create branch {}: {}", i, _e);
            continue;
        }

        let file_path = branch_dir.join(format!("data_{:02}.txt", i));
        match File::create(&file_path) {
            Ok(mut f) => {
                if f.write_all(b"branch data").is_ok() {
                    branches_created += 1;
                }
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D9: Failed to create file in branch {}: {}", i, _e);
            }
        }
    }

    // Also place a file in the root of demo_subdir for reference
    if let Ok(mut f) = File::create(demo_subdir.join("root_file.txt")) {
        let _ = f.write_all(b"root level data");
    }

    println!(
        "   Setup: {} branches created, each with 1 file",
        branches_created
    );
    println!("   Setup: 1 file at root level");
    println!("   Total expected files: {}", branches_created + 1);

    // -- Walk 1: Unlimited queue (baseline) --
    println!("\n   Walk A: Unlimited queue (baseline)");

    let unlimited_config = WalkConfig::new()
        .yield_directories(false)
        .continue_on_error(true);

    let unlimited_walker = DirWalker::new(&demo_subdir, unlimited_config);
    let unlimited_file_count = unlimited_walker.filter_map(|r| r.ok()).count();

    println!("   Files found (unlimited): {}", unlimited_file_count);

    // -- Walk 2: Queue limited to 3 --
    println!("\n   Walk B: max_queue_size = 3");

    let limited_config = WalkConfig::new()
        .max_queue_size(3)
        .yield_directories(false)
        .continue_on_error(true);

    let limited_walker = DirWalker::new(&demo_subdir, limited_config);

    let mut limited_file_count: usize = 0;

    for entry_result in limited_walker {
        match entry_result {
            Ok(entry) => {
                let display_name = entry.file_name().unwrap_or("<non-utf8>");
                let depth_indent = "  ".repeat(entry.depth());
                println!(
                    "   {}📄 {} [depth:{}]",
                    depth_indent,
                    display_name,
                    entry.depth()
                );
                limited_file_count += 1;
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("D9: Walk error (continuing): {}", _e);
            }
        }
    }

    println!("\n📊 Results:");
    println!("   Files found (unlimited queue): {}", unlimited_file_count);
    println!("   Files found (queue limit = 3): {}", limited_file_count);
    println!("   Queue limit:                   3");

    if limited_file_count < unlimited_file_count {
        println!(
            "   ✓ Queue limit caused {} branches to be skipped",
            unlimited_file_count.saturating_sub(limited_file_count)
        );
        println!("     (Subdirectories beyond queue capacity were not enqueued)");
    } else if limited_file_count == unlimited_file_count {
        println!("   ℹ Queue limit did not cause skipping in this case");
        println!("     (All directories fit within queue as they were processed)");
    }

    // -- Walk 3: Queue limited to 3 with continue_on_error=false --
    // This shows that QueueSizeExceeded errors are returned when not continuing
    println!("\n   Walk C: max_queue_size = 3, continue_on_error = false");

    let strict_config = WalkConfig::new()
        .max_queue_size(3)
        .yield_directories(true)
        .continue_on_error(false);

    let strict_walker = DirWalker::new(&demo_subdir, strict_config);

    let mut strict_ok_count: usize = 0;
    let mut strict_err_count: usize = 0;
    let mut stopped_with_error = false;

    for entry_result in strict_walker {
        match entry_result {
            Ok(_entry) => {
                strict_ok_count += 1;
            }
            Err(_e) => {
                strict_err_count += 1;
                stopped_with_error = true;

                #[cfg(debug_assertions)]
                println!("   ❌ Fatal error returned: {}", _e);

                #[cfg(not(debug_assertions))]
                println!("   ❌ Fatal error returned (walk halted)");

                // After a fatal error with continue_on_error=false,
                // the iterator returns None on subsequent calls.
                // The for loop will terminate naturally.
            }
        }
    }

    println!("   Entries before halt: {}", strict_ok_count);
    println!("   Errors returned:     {}", strict_err_count);
    if stopped_with_error {
        println!("   ✓ Walk correctly halted on queue overflow in strict mode");
    } else {
        println!("   ℹ No queue overflow occurred in strict mode");
    }

    println!();

    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

/// Entry point: create test structure and run all demos.
///
/// ## Error Handling Strategy
/// Each demo is run independently. If one demo fails, the error is
/// logged (debug builds only) and execution continues to the next demo.
/// main() itself never panics — it returns Ok(()) unconditionally after
/// attempting all demos.
///
/// ## Cleanup
/// Test directory is left intact for manual inspection.
/// Cleanup instructions are printed at the end.
fn main() -> Result<(), DemoError> {
    println!("\n╔═══════════════════════════════════════════════════════════╗");
    println!("║  Directory Walk Module - Comprehensive Demo               ║");
    println!("║  Zero Dependencies | Production Safe | Cross-Platform     ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    let test_dir = std::env::temp_dir().join("directory_walk_demo_test");

    // Production-safe: do not print full temp_dir path in release builds
    #[cfg(debug_assertions)]
    println!("Test directory location: {}", test_dir.display());

    #[cfg(not(debug_assertions))]
    println!("Test directory: (location hidden in release build)");

    println!("(You can inspect this directory manually after the demo.)\n");

    // Clean up any existing test directory from previous runs
    if test_dir.exists() {
        match fs::remove_dir_all(&test_dir) {
            Ok(()) => println!("🧹 Cleaned up existing test directory\n"),
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("MAIN: Could not clean existing directory: {}", _e);
                // Continue anyway — create_dir_all may still succeed
            }
        }
    }

    // Create test structure — if this fails, no demos can run
    if let Err(_e) = create_test_directory_structure(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Failed to create test directory structure: {}", _e);

        println!("❌ Could not create test directory. Demos cannot run.");
        return Ok(()); // Do not halt/panic — return gracefully
    }

    // Run each demo independently — errors are caught per-demo so that
    // one failing demo does not prevent the others from running.

    if let Err(_e) = demo_basic_recursive_walk(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 1 failed: {}", _e);
        println!("⚠ Demo 1 encountered an issue, continuing...\n");
    }

    if let Err(_e) = demo_max_depth_walk(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 2 failed: {}", _e);
        println!("⚠ Demo 2 encountered an issue, continuing...\n");
    }

    if let Err(_e) = demo_files_only_walk(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 3 failed: {}", _e);
        println!("⚠ Demo 3 encountered an issue, continuing...\n");
    }

    if let Err(_e) = demo_extension_filter_walk(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 4 failed: {}", _e);
        println!("⚠ Demo 4 encountered an issue, continuing...\n");
    }

    if let Err(_e) = demo_collect_and_sort(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 5 failed: {}", _e);
        println!("⚠ Demo 5 encountered an issue, continuing...\n");
    }

    // Demo 6 returns a hash value — we discard it here since this is a demo
    if let Err(_e) = demo_directory_hash(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 6 failed: {}", _e);
        println!("⚠ Demo 6 encountered an issue, continuing...\n");
    }

    if let Err(_e) = demo_error_handling_patterns(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 7 failed: {}", _e);
        println!("⚠ Demo 7 encountered an issue, continuing...\n");
    }

    // Demo 7

    if let Err(_e) = demo_max_entries_per_dir(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 8 failed: {}", _e);
        println!("⚠ Demo 8 encountered an issue, continuing...\n");
    }

    if let Err(_e) = demo_max_queue_size(&test_dir) {
        #[cfg(debug_assertions)]
        eprintln!("MAIN: Demo 9 failed: {}", _e);
        println!("⚠ Demo 9 encountered an issue, continuing...\n");
    }

    // Final summary
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ All Demos Completed!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    println!("\n📂 Test Directory:");
    println!("   Status: Left intact for manual inspection");

    // Cleanup instructions: show path only in debug builds
    println!("\n🧹 Manual Cleanup:");
    #[cfg(debug_assertions)]
    {
        #[cfg(target_os = "windows")]
        println!("   rmdir /s \"{}\"", test_dir.display());
        #[cfg(not(target_os = "windows"))]
        println!("   rm -rf \"{}\"", test_dir.display());
    }

    #[cfg(not(debug_assertions))]
    println!("   Remove the test directory from your system temp folder");

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    Ok(())
}
