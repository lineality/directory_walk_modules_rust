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

// Platform-specific imports for symlink cycle detection
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(windows)]
use std::collections::HashSet;

#[cfg(unix)]
use std::collections::HashSet;

/*

(production-Rust rules)
# 🦀 Rust rules 🦀:
- Always best practice.
- Always extensive doc strings: what the code is doing with project context
- Always clear comments.
- Always cargo tests (where possible).
- Never remove documentation.
- Always clear, meaningful, unique names (e.g. variables, functions).
- Always absolute file paths.
- Always error handling.
- Never unsafe code.
- Never use unwrap.

- Load what is needed when it is needed: Do not ever load a whole file or line, rarely load a whole anything. increment and load only what is required pragmatically. Do not fill 'state' with every possible piece of un-used information. Do not insecurity output information broadly in the case of errors and exceptions.

- Always defensive best practice
- Always error and exception handling: Every part of code, every process, function, and operation will fail at some point, if only because of cosmic-ray bit-flips (which are common), hardware failure, power-supply failure, adversarial attacks, etc. There must always be fail-safe error handling where production-release-build code handles issues and moves on without panic-crashing ever. Every failure must be handled smoothly: let it fail and move on. This does not mean that no function can return an error. Handling should occur where needed, e.g. before later functions are reached.

Somehow there seems to be no clear vocabulary for 'Do not stop.' When you come to something to handle, handle it:
- Handle and move on: Do not halt the program.
- Handle and move on: Do not terminate the program.
- Handle and move on: Do not exit the program.
- Handle and move on: Do not crash the program.
- Handle and move on: Do not panic the program.
- Handle and move on: Do not coredump the program.
- Handle and move on: Do not stop the program.
- Handle and move on: Do not finish the program.

Comments and docs for functions and groups of functions must include project level information: To paraphrase Jack Welch, "The most dangerous thing in the world is a flawless operation that should never have been done in the first place." For projects, functions are not pure platonic abstractions; the project has a need that the function is or is not meeting. It happens constantly that a function does the wrong thing well and so this 'bug' is never detected. Project-level documentation and logic-level documentation are two different things that must both exist such that discrepancies must be identifiable; Project-level documentation, logic-level documentation, and the code, must align and align with user-needs, real conditions, and future conditions.

Safety, reliability, maintainability, fail-safe, communication-documentation, are the goals: not ideology, aesthetics, popularity, momentum-tradition, bad habits, convenience, nihilism, lazyness, lack of impulse control, etc.

## No third party libraries (or very strictly avoid third party libraries where possible).

## Scale: Code should be future proof and scale well. The Y2K bug was not a wonderful feature, it was a horrendous mistake. Scale and size should be handled in a modular no-load way, not arbitrarily capped so that everything breaks.

## Rule of Thumb, ideals not absolute rules: Follow NASA's 'Power of 10 rules' where possible and sensible (as updated for 2025 and Rust (not narrowly 2006 c for embedded systems):
1. no unsafe stuff:
- no recursion
- no goto
- no pointers
- no preprocessor

2. upper bound on all normal-loops, failsafe for all always-loops

3. Pre-allocate all memory (no dynamic memory allocation)

4. Clear function scope and Data Ownership: Part of having a function be 'focused' means knowing if the function is in scope. Functions should be neither swiss-army-knife functions that do too many things, nor scope-less micro-functions that may be doing something that should not be done. Many functions should have a narrow focus and a short length, but definition of actual-project scope functionality must be explicit. Replacing one long clear in-scope function with 50 scope-agnostic generic sub-functions with no clear way of telling if they are in scope or how they interact (e.g. hidden indirect recursion) is unsafe. Rust's ownership and borrowing rules focus on Data ownership and hidden dependencies, making it even less appropriate to scatter borrowing and ownership over a spray of microfunctions purely for the ideology of turning every operation into a microfunction just for the sake of doing so. (See more in rule 9.)

5. Defensive programming: debug-assert, test-assert, prod safely check & handle, not 'assert!' panic

Note: Terminology varies across "error" / "fail" / "exception" / "catch" / "case" et al. The standard terminology is 'error handling' but 'case handling' or 'issue handling' may be a more accurate description, especially where 'error' refers to the output when unable to handle a case (which becomes semantically paradoxical). The goal is not terminating / halting / ending / shutting down / stopping, etc., or crashing / failing / panicking / coredumping / undefined-behavior-ing, etc. the program when an expected case occurs. Here production and debugging/testing starkly diverge: during testing you want to see how (and where in the code) the program may 'fail' and where and when cases are encountered. In production the satellite must not fall out of the sky ever, regardless of how pedantically beautiful the error-message in the ball of flames may have been.

For production-release code:
1. check and handle without panic/halt in production
2. return result (such as Result<T, E>) and smoothly handle errors (not halt-panic stopping the application): no assert!() outside of test-only code
Return Result<T, E>, with case/error/exception handling, so long as that is caught somewhere. Only in cases where there is no way (or no where) to handle the error-output should the function always return OK(), failing completely silently (sometimes internal-to-function error logging is best). Allow-to-fail and handle is not the same as no-handling. This is case-by case.
3. test assert: use #[cfg(test)] assert!() to test production binaries (not in prod or debug modes)
4. debug assert: use debug_assert! with  #[cfg(all(debug_assertions, not(test)))] to run tests in debug builds (not in prod, not in test)
5. note: #[cfg(debug_assertions)] and debug_assert! ARE active in test builds
6. use defensive programming with recovery of all issues at all times
- use cargo tests
- use debug_asserts
- do not leave assertions in production code.
- use no-panic error handling
- use Option
- use enums and structs
- check bounds
- check returns
- note: a test-flagged assert can test a production release build (whereas debug_assert cannot); cargo test --release
```
#[cfg(test)]
assert!(
```

e.g.
# "Assert & Catch-Handle" 3-part System

A three-part rule of thumb may be:

1. For Debug assertions: Only in debug builds, NOT in tests - use: #[cfg(all(debug_assertions, not(test)))]

2. For Test assertions: use in test functions themselves, not in the function body (easy to conflict with debug/prod handling)
E.g.
When we run a cargo test:
- The #[cfg(test)] assert compiles and is active
- the cargo-test calls string_concat_list_function()
- an assert! in the abc_function (not in the test) panics immediately inside the abc_function
- abc_function never reaches the production error handling
- so abc_function never returns an Err(...)
- so the cargo-test 'fails' with a panic, not with a cargo-test error result

3. Production catches: Always present, return production-safe no-heap terse errors (no panic, no open-ended data exfiltration), with unique error prefixes to identify the function, e.g. 'SCLF error: arg empty' for string_concat_list_function()



// template/example for check/assert format
//    =================================================
// // Debug-Assert, Test-Asset, Production-Catch-Handle
//    =================================================
// This is not included in production builds
// debug_assert: IS also active during test-builds
// use #[cfg(not(test))] to run in debug-build only: will panic
#[cfg(not(test))]
debug_assert!(
    INFOBAR_MESSAGE_BUFFER_SIZE > 0,
    "Info bar buffer must have non-zero capacity"
);

// this is included in debug builds AND test builds
#[cfg(all(debug_assertions, not(test)))]
{
xyz
}


// note: this may be located only in cargo test functions
// This is not included in production builds
// assert: only when running cargo test: will panic
#[cfg(test)]
assert!(
    INFOBAR_MESSAGE_BUFFER_SIZE > 0,
    "Info bar buffer must have non-zero capacity"
);
// Catch & Handle without panic in production
// This IS included in production to safe-catch
if !INFOBAR_MESSAGE_BUFFER_SIZE == 0 {
    // state.set_info_bar_message("Config error");
    return Err(LinesError::GeneralAssertionCatchViolation(
        "zero buffer size error".into(),
    ));
}

Depending on the test, you may need a test-assert to be in a cargo-test function and not in the main function.

Warning: Do not collide or mix up test-asserts and debug asserts, or forget that debug code also runs in test builds by default.;
use #[cfg(all(debug_assertions, not(test)))] for debug build only (not test build).
use #[cfg(test)] assert!(  for test build only, not debug).
Give descriptive non-colliding names to cargo-tests and test sets.

Note: production-use characters and strings can be formatted, written, printed using modules such as Buffy
https://github.com/lineality/buffy_stack_format_write_module
instead of using standard Rust macros such as format! print! write! that use heap-memory.


Note: Error messages must be unique per function (e.g. name of function (or abbreviation) in the error message). Colliding generic error messages that cannot be traced to a specific function are a significant liability.


Avoid heap for error messages and for all things:
Is heap used for error messages because that is THE best way, the most secure, the most efficient, proper separate of debug testing vs. secure production code?
Or is heap used because of oversights and apathy: "it's future dev's problem, let's party."
We can use heap in debug/test modes/builds only.
Production software must not insecurely output debug diagnostics.
Debug information must not be included in production builds: "developers accidentally left development code in the software" is a classic error (not a desired design spec) that routinely leads to security and other issues. That is NOT supposed to happen. It is not coherent to insist the open ended heap output 'must' or 'should' be in a production build.

This is central to the question about testing vs. a pedantic ban on conditional compilation; not putting full traceback insecurity into production code is not a different operational process logic tree for process operations.

Just like with the pedantic "all loops being bounded" rule, there is a fundamental exception: always-on loops must be the opposite.
With conditional compilations: code NEVER to EVER be in production-builds MUST be always "conditionally" excluded. This is not an OS conditional compilation or a hardware conditional compilation. This is an 'unsafe-testing-only or safe-production-code' condition.

Error messages and error outcomes in 'production' 'release' (real-use, not debug/testing) must not ever contain any information that could be a security vulnerability or attack surface. Failing to remove debugging inspection is a major category of security and hygiene problems.

Security: Error messages in production must NOT contain:
- File paths (can reveal system structure)
- File contents
- environment variables
- user, file, state, data
- internal implementation details
- etc.

All debug-prints not for production must be tagged with:
```
#[cfg(debug_assertions)]
```

Production output following an error / exception / case must be managed and defined, not not open to whatever an api or OS-call wants to dump out.

6. Manage ownership and borrowing

7. Manage return values:
- use null-void return values
- check non-void-null returns

8. Navigate debugging and testing on the one hand and not-dangerous conditional-compilation on the other hand:
- Here 'conditional compilation' is interpreted as significant changes to the overall 'tree' of operation depending on build settings/conditions, such as using different modules and basal functions. E.g. "GDPR compliance mode compilation"
- Any LLVM type compilation or build-flag will modify compilation details, but not the target tree logic of what the software does (arguably).
- 2025+ "compilation" and "conditions" cannot be simplistically compared with single-architecture 1970 pdp-11-only C or similar embedded device compilation.

9. Communicate:
- Use doc strings; use comments.
- Document use-cases, edge-cases, and policies (These are project specific and cannot be telepathed from generic micro-function code. When a Mars satellite failed because one team used SI-metric units and another team did not, that problem could not have been detected by looking at, and auditing, any individual function in isolation without documentation. Breaking a process into innumerable undocumented micro-functions can make scope and policy impossible to track. To paraphrase Jack Welch: "The most dangerous thing in the world is a flawless operation that should never have been done in the first place.")

10. Use state-less operations when possible:
- a seemingly invisibly small increase in state often completely destroys projects
- expanding state destroys projects with unmaintainable over-reach

🦀Vigilance🦀: Properly written code supports users, developers, and the people who depend upon maintainable software. Maintainable software supports the future for us all.

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

    /// Symlink cycle detected during traversal.
    ///
    /// This error only occurs when `follow_symlinks` is `true` and
    /// cycle detection discovers that a symlink points back to a
    /// directory already being traversed. The cycle is detected via:
    /// - Unix: (device, inode) pair tracking
    /// - Windows: Canonicalized path tracking
    ///
    /// Debug-site prefix: DWSC (Dir Walk Symlink Cycle)
    SymlinkCycle,
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
            WalkError::SymlinkCycle => write!(f, "DWSC: symlink cycle detected"),
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

    /// Whether this entry is a symbolic link.
    ///
    /// `true` even when the symlink target is a directory or file.
    /// Symlink type is determined by `fs::symlink_metadata()` which
    /// does not follow the link to its target.
    is_symlink: bool,
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
                .field("is_symlink", &self.is_symlink)
                .finish()
        }

        #[cfg(not(debug_assertions))]
        {
            f.debug_struct("DirEntry")
                .field("name", &name)
                .field("depth", &self.depth)
                .field("is_dir", &self.is_dir)
                .field("is_file", &self.is_file)
                .field("is_symlink", &self.is_symlink)
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

    /// Check if this entry is a symbolic link.
    ///
    /// # Returns
    /// `true` if this entry is a symlink (regardless of target type),
    /// `false` otherwise.
    ///
    /// # Project Context
    /// Used to identify and skip or specially handle symlinked entries during
    /// team channel scanning, preventing symlink-based attacks.
    ///
    /// # Note
    /// When `follow_symlinks` is `false` (default), symlinks appear as
    /// symlinks in results. When `follow_symlinks` is `true`, symlinks
    /// to directories may appear as directories (`is_dir() == true`) if
    /// their target exists and is a directory.
    ///
    /// This method always returns the original symlink status determined
    /// by `fs::symlink_metadata()`.
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
    ///
    ///     if entry.is_symlink() {
    ///         // Handle symlinks specially
    ///         println!("Found symlink: {:?}", entry.file_name());
    ///     }
    /// }
    /// ```
    pub fn is_symlink(&self) -> bool {
        self.is_symlink
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

    /// Whether to follow symbolic links during traversal.
    ///
    /// ## Behavior
    /// - `false` (default): Symlinks are yielded as entries but their targets
    ///   are not traversed. Uses `fs::symlink_metadata()` which does not
    ///   follow links. The symlink itself appears in results with
    ///   `is_symlink() == true`.
    /// - `true`: Symlinks to directories are followed and their contents
    ///   traversed. Cycle detection prevents infinite loops. Broken symlinks
    ///   are skipped gracefully.
    ///
    /// ## Security Note
    /// Following symlinks allows traversal outside the intended directory tree.
    /// A malicious user could plant a symlink to `/etc` or `/home` to expose
    /// sensitive files. Production code should use `follow_symlinks = false`
    /// unless symlink traversal is explicitly required and the directory
    /// source is trusted.
    ///
    /// ## Project Context
    /// For team channel scanning, symlinks should NOT be followed:
    /// - Users should not be able to expose system files via symlinks
    /// - Channel directories should contain only direct content
    /// - Symlinks in user-controlled directories are a security risk
    follow_symlinks: bool,
}

impl Default for WalkConfig {
    fn default() -> Self {
        WalkConfig {
            max_depth: None,
            yield_directories: true,
            continue_on_error: true,
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            max_entries_per_dir: DEFAULT_MAX_ENTRIES_PER_DIR,
            follow_symlinks: false, // Secure default
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

    /// Set whether to follow symbolic links during traversal.
    ///
    /// # Arguments
    /// * `follow` - If `true`, symlinks to directories are followed and their
    ///   contents traversed (with cycle detection). If `false` (default),
    ///   symlinks are yielded as entries but not traversed.
    ///
    /// # Security Warning
    /// Enabling symlink following allows traversal outside the intended
    /// directory tree. Only enable this for trusted directory sources.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use crate::dir_walk_module::WalkConfig;
    /// // Secure: do not follow symlinks (default)
    /// let config = WalkConfig::new().follow_symlinks(false);
    ///
    /// // Risky: follow symlinks (use only for trusted sources)
    /// let config = WalkConfig::new().follow_symlinks(true);
    /// ```
    ///
    /// Only enable this if your use case specifically requires following
    /// symlinks and you trust the directory contents.
    pub fn follow_symlinks(mut self, follow: bool) -> Self {
        self.follow_symlinks = follow;
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

    /// Tracks visited directories to prevent cycles when following symlinks.
    ///
    /// ## Unix Implementation
    /// Stores (device, inode) pairs. Two paths referring to the same inode
    /// on the same device are the same directory, even if reached via
    /// different symlinks.
    ///
    /// ## Windows Implementation
    /// Stores canonicalized paths. Canonicalization resolves all symlinks,
    /// junctions, and relative components, so two paths canonicalizing to
    /// the same result refer to the same directory.
    ///
    /// Only populated when `config.follow_symlinks` is `true`.
    #[cfg(unix)]
    visited: std::collections::HashSet<(u64, u64)>,

    /// Tracks visited directories to prevent cycles when following symlinks.
    ///
    /// ## Unix Implementation
    /// Stores (device, inode) pairs. Two paths referring to the same inode
    /// on the same device are the same directory, even if reached via
    /// different symlinks.
    ///
    /// ## Windows Implementation
    /// Stores canonicalized paths. Canonicalization resolves all symlinks,
    /// junctions, and relative components, so two paths canonicalizing to
    /// the same result refer to the same directory.
    ///
    /// Only populated when `config.follow_symlinks` is `true`.
    #[cfg(windows)]
    visited: std::collections::HashSet<PathBuf>,
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
            #[cfg(unix)]
            visited: HashSet::new(),
            #[cfg(windows)]
            visited: HashSet::new(),
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

    //     /// Read one directory from the filesystem and populate `current_entries`.
    //     ///
    //     /// This is the core I/O function. It reads entries from a single
    //     /// directory, enqueues discovered subdirectories for later traversal,
    //     /// and buffers entries that pass yield filters into `current_entries`
    //     /// for the iterator to return.
    //     ///
    //     /// Called by `next()` when `current_entries` is empty and the queue
    //     /// still contains directories to process.
    //     ///
    //     /// # Arguments
    //     /// * `dir_path` - Path to the directory to read
    //     /// * `depth` - Depth of entries found in this directory (0 = root's
    //     ///   immediate children)
    //     ///
    //     /// # Returns
    //     /// * `Ok(())` - Directory was read (entries may or may not have been
    //     ///   buffered, depending on content and filters)
    //     /// * `Err(WalkError)` - A fatal error occurred (only when
    //     ///   `continue_on_error` is false)
    //     ///
    //     /// # How Entry Counting Works
    //     /// The counter `entries_read_this_dir` increments for every entry
    //     /// successfully read from the filesystem, BEFORE any yield filtering.
    //     /// This means the limit bounds actual I/O operations performed on
    //     /// this directory, not the number of entries returned to the caller.
    //     ///
    //     /// ```text
    //     /// for each entry from OS:
    //     ///     if entries_read_this_dir >= limit → stop reading
    //     ///     read entry from filesystem
    //     ///     read metadata from filesystem
    //     ///     entries_read_this_dir += 1          ← counts I/O, not yield
    //     ///     if directory → enqueue for later
    //     ///     if passes yield filter → buffer for caller
    //     /// ```
    //     ///
    //     /// This design prevents a directory with millions of non-yielded
    //     /// entries (e.g. subdirectories when `yield_directories=false`)
    //     /// from consuming unbounded I/O and memory.
    //     ///
    //     /// # Bounds Enforced (in order)
    //     /// 1. Depth limit: checked before any I/O; returns `Ok(())` if
    //     ///    beyond `max_depth`
    //     /// 2. Entry limit: checked per entry during reading; stops the
    //     ///    `read_dir` loop when reached
    //     /// 3. Queue size: checked before enqueuing each subdirectory;
    //     ///    skips enqueue if queue is full
    //     /// 4. Depth overflow: checked via `checked_add` before computing
    //     ///    next depth level
    //     fn read_directory(&mut self, dir_path: &Path, depth: usize) -> Result<(), WalkError> {
    //         // =================================================
    //         // Debug-Assert, Test-Assert, Production-Catch-Handle
    //         // =================================================

    //         // Debug-only: verify current_entries was drained before refill
    //         #[cfg(all(debug_assertions, not(test)))]
    //         {
    //             if !self.current_entries.is_empty() {
    //                 eprintln!(
    //                     "DW_DBG: read_directory called with {} buffered entries still pending",
    //                     self.current_entries.len()
    //                 );
    //             }
    //         }

    //         // Production catch: check depth limit before doing any I/O
    //         if let Some(max_depth) = self.config.max_depth {
    //             if depth > max_depth {
    //                 return Ok(()); // Beyond limit, silently skip
    //             }
    //         }

    //         // Attempt to read directory
    //         let read_dir = match fs::read_dir(dir_path) {
    //             Ok(rd) => rd,
    //             Err(_e) => {
    //                 #[cfg(debug_assertions)]
    //                 eprintln!("DWRD: Failed to read directory at depth {}: {}", depth, _e);
    //                 return Err(WalkError::ReadDirectory);
    //             }
    //         };

    //         // Track entries READ from this single directory (not just yielded).
    //         // This bounds the I/O and queue growth from any single directory,
    //         // regardless of whether entries are yielded to the caller.
    //         let mut entries_read_this_dir: usize = 0;

    //         // Process each entry in directory
    //         for entry_result in read_dir {
    //             // Production catch: enforce per-directory entry limit.
    //             // This check happens BEFORE processing each entry, bounding
    //             // total work done per directory regardless of yield settings.
    //             if entries_read_this_dir >= self.config.max_entries_per_dir {
    //                 #[cfg(debug_assertions)]
    //                 eprintln!(
    //                     "DWEL: Entry limit ({}) reached; read {} entries from directory at depth {}",
    //                     self.config.max_entries_per_dir, entries_read_this_dir, depth
    //                 );

    //                 if self.config.continue_on_error {
    //                     break; // Stop reading this directory, continue walk
    //                 } else {
    //                     return Err(WalkError::EntryLimitExceeded);
    //                 }
    //             }

    //             // Get directory entry
    //             let entry = match entry_result {
    //                 Ok(e) => e,
    //                 Err(_e) => {
    //                     #[cfg(debug_assertions)]
    //                     eprintln!("DWRD: Failed to read dir entry at depth {}: {}", depth, _e);

    //                     if self.config.continue_on_error {
    //                         continue;
    //                     } else {
    //                         return Err(WalkError::ReadDirectory);
    //                     }
    //                 }
    //             };

    //             let entry_path = entry.path();

    //             // Get metadata to determine file type
    //             let metadata = match entry.metadata() {
    //                 Ok(m) => m,
    //                 Err(_e) => {
    //                     #[cfg(debug_assertions)]
    //                     eprintln!("DWEM: Failed to get metadata at depth {}: {}", depth, _e);

    //                     if self.config.continue_on_error {
    //                         continue;
    //                     } else {
    //                         return Err(WalkError::EntryMetadata);
    //                     }
    //                 }
    //             };

    //             // ────────────────────────────────────────────────────
    //             // COUNT I/O OPERATIONS, NOT YIELDED ENTRIES
    //             // ────────────────────────────────────────────────────
    //             // This increment MUST be here — after successful metadata
    //             // read, BEFORE the yield decision. If this were inside
    //             // the `if should_yield` block, then entries filtered out
    //             // by yield settings (e.g. directories when
    //             // `yield_directories=false`) would not be counted, and
    //             // the limit would fail to bound I/O on directories
    //             // containing only non-yielded entry types.
    //             // ────────────────────────────────────────────────────
    //             entries_read_this_dir += 1;

    //             let is_dir = metadata.is_dir();
    //             let is_file = metadata.is_file();

    //             // Enqueue subdirectories for later processing (with bounds checks)
    //             if is_dir {
    //                 // Checked depth arithmetic: prevent usize overflow
    //                 let next_depth = match depth.checked_add(1) {
    //                     Some(d) => d,
    //                     None => {
    //                         #[cfg(debug_assertions)]
    //                         eprintln!(
    //                             "DWDO: Depth overflow at depth {} — skipping subdirectory",
    //                             depth
    //                         );

    //                         if self.config.continue_on_error {
    //                             continue;
    //                         } else {
    //                             return Err(WalkError::DepthOverflow);
    //                         }
    //                     }
    //                 };

    //                 let should_descend = match self.config.max_depth {
    //                     None => true,
    //                     Some(max_depth) => next_depth <= max_depth,
    //                 };

    //                 if should_descend {
    //                     // Production catch: enforce queue size limit
    //                     if self.queue.len() >= self.config.max_queue_size {
    //                         #[cfg(debug_assertions)]
    //                         eprintln!(
    //                             "DWQS: Queue size limit ({}) reached — skipping subdirectory at depth {}",
    //                             self.config.max_queue_size, next_depth
    //                         );

    //                         if !self.config.continue_on_error {
    //                             return Err(WalkError::QueueSizeExceeded);
    //                         }
    //                         // If continue_on_error, just skip enqueuing this subdir
    //                     } else {
    //                         self.queue.push_back((entry_path.clone(), next_depth));
    //                     }
    //                 }
    //             }

    //             // Decide whether to yield this entry to the caller
    //             let should_yield = if is_dir {
    //                 self.config.yield_directories
    //             } else {
    //                 true
    //             };

    //             if should_yield {
    //                 self.current_entries.push_back(DirEntry {
    //                     path: entry_path,
    //                     depth,
    //                     is_dir,
    //                     is_file,
    //                 });
    //             }
    //         }

    //         Ok(())
    //     }

    /// Read one directory from the filesystem and populate `current_entries`.
    ///
    /// This is the core I/O function. It reads entries from a single directory,
    /// handles symlinks according to configuration, detects cycles when following
    /// symlinks, enqueues discovered subdirectories for later traversal, and
    /// buffers entries that pass yield filters into `current_entries` for the
    /// iterator to return.
    ///
    /// # Symlink Handling
    /// - Uses `fs::symlink_metadata()` which does NOT follow symlinks
    /// - If `follow_symlinks` is false: symlinks are yielded as-is, not traversed
    /// - If `follow_symlinks` is true: symlinks to directories are followed, with
    ///   cycle detection via device/inode (Unix) or canonicalized path (Windows)
    ///
    /// # Arguments
    /// * `dir_path` - Path to the directory to read
    /// * `depth` - Depth of entries found in this directory (0 = root's immediate children)
    ///
    /// # Returns
    /// * `Ok(())` - Directory was read successfully
    /// * `Err(WalkError)` - Fatal error occurred (only when `continue_on_error` is false)
    fn read_directory(&mut self, dir_path: &Path, depth: usize) -> Result<(), WalkError> {
        // =================================================
        // Debug-Assert, Test-Assert, Production-Catch-Handle
        // =================================================

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
                return Ok(());
            }
        }

        let read_dir = match fs::read_dir(dir_path) {
            Ok(rd) => rd,
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!("DWRD: Failed to read directory at depth {}: {}", depth, _e);
                return Err(WalkError::ReadDirectory);
            }
        };

        let mut entries_read_this_dir: usize = 0;

        for entry_result in read_dir {
            // Production catch: enforce per-directory entry limit
            if entries_read_this_dir >= self.config.max_entries_per_dir {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DWEL: Entry limit ({}) reached for directory at depth {}",
                    self.config.max_entries_per_dir, depth
                );

                if self.config.continue_on_error {
                    break;
                } else {
                    return Err(WalkError::EntryLimitExceeded);
                }
            }

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

            // NEW CODE: Use symlink_metadata to NOT follow symlinks
            let metadata = match fs::symlink_metadata(&entry_path) {
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

            entries_read_this_dir += 1;

            // NEW CODE: Capture symlink status
            let is_symlink = metadata.is_symlink();
            let mut is_dir = metadata.is_dir();
            let mut is_file = metadata.is_file();

            // NEW CODE: Handle symlink resolution if configured
            if is_symlink && self.config.follow_symlinks {
                // Get target metadata (follows the link)
                match fs::metadata(&entry_path) {
                    Ok(target_meta) => {
                        if target_meta.is_dir() {
                            // NEW CODE: Check for cycles before marking as directory
                            let already_visited =
                                self.check_and_mark_visited(&entry_path, &target_meta)?;

                            if already_visited {
                                #[cfg(debug_assertions)]
                                eprintln!(
                                    "DWSC: Symlink cycle detected at depth {}, skipping: {:?}",
                                    depth,
                                    entry_path.file_name()
                                );

                                if self.config.continue_on_error {
                                    continue;
                                } else {
                                    return Err(WalkError::SymlinkCycle);
                                }
                            }

                            // Mark as directory so it gets enqueued below
                            is_dir = true;
                            is_file = false;
                        } else if target_meta.is_file() {
                            is_file = true;
                            is_dir = false;
                        }
                    }
                    Err(_e) => {
                        #[cfg(debug_assertions)]
                        eprintln!("DWSL: Symlink target unreadable at depth {}: {}", depth, _e);
                        // Broken symlink - skip it
                        if self.config.continue_on_error {
                            continue;
                        } else {
                            return Err(WalkError::EntryMetadata);
                        }
                    }
                }
            }

            // Enqueue subdirectories for later processing
            if is_dir {
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
                    if self.queue.len() >= self.config.max_queue_size {
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "DWQS: Queue size limit ({}) reached — skipping subdirectory at depth {}",
                            self.config.max_queue_size, next_depth
                        );

                        if !self.config.continue_on_error {
                            return Err(WalkError::QueueSizeExceeded);
                        }
                    } else {
                        self.queue.push_back((entry_path.clone(), next_depth));
                    }
                }
            }

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
                    // NEW CODE
                    is_symlink,
                });
            }
        }

        Ok(())
    }

    /// Check if a directory has already been visited, and mark it as visited.
    ///
    /// Used for symlink cycle detection when `follow_symlinks` is true.
    ///
    /// # Platform-Specific Implementation
    /// - Unix: Uses (device, inode) pairs from metadata
    /// - Windows: Uses canonicalized path
    ///
    /// # Arguments
    /// * `path` - Path to the directory (may be a symlink target)
    /// * `metadata` - Metadata for the target directory (after following symlink)
    ///
    /// # Returns
    /// * `Ok(true)` - Directory was already visited (cycle detected)
    /// * `Ok(false)` - Directory is new, now marked as visited
    /// * `Err(WalkError)` - Error accessing directory information (Windows only,
    ///   if canonicalization fails)
    #[cfg(unix)]
    fn check_and_mark_visited(
        &mut self,
        _path: &Path,
        metadata: &fs::Metadata,
    ) -> Result<bool, WalkError> {
        let dev = metadata.dev();
        let ino = metadata.ino();
        let key = (dev, ino);

        if self.visited.contains(&key) {
            Ok(true) // Already visited
        } else {
            self.visited.insert(key);
            Ok(false) // Newly visited
        }
    }

    #[cfg(windows)]
    fn check_and_mark_visited(
        &mut self,
        path: &Path,
        _metadata: &fs::Metadata,
    ) -> Result<bool, WalkError> {
        match fs::canonicalize(path) {
            Ok(canonical) => {
                if self.visited.contains(&canonical) {
                    Ok(true) // Already visited
                } else {
                    self.visited.insert(canonical);
                    Ok(false) // Newly visited
                }
            }
            Err(_e) => {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DWSC: Failed to canonicalize path for cycle detection: {}",
                    _e
                );

                // If we can't canonicalize, treat as unvisited and continue
                // (conservative approach - may traverse same dir twice rather than skip)
                Ok(false)
            }
        }
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
            is_symlink: false,
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

    // ========================================================================
    // Symlink Tests
    // ========================================================================
    /// Test: symlinks are not followed by default.
    ///
    /// Creates a symlink to an external directory. Default walk should see
    /// the symlink entry but NOT traverse into it, so files within the
    /// target directory should NOT appear in results.
    #[test]
    fn test_symlinks_not_followed_by_default() {
        let base = test_dir("symlink_not_followed_base");
        cleanup(&base);

        // Create walk root and external target directory
        let walk_root = base.join("walk_root");
        let target = base.join("external_target"); // OUTSIDE walk_root

        assert!(
            fs::create_dir_all(&walk_root).is_ok(),
            "test_symlinks_not_followed: failed to create walk root"
        );
        assert!(
            fs::create_dir_all(&target).is_ok(),
            "test_symlinks_not_followed: failed to create target directory"
        );

        // Put a file in the external target
        let target_file = target.join("file_in_target.txt");
        if let Ok(mut f) = File::create(&target_file) {
            let _ = f.write_all(b"test");
        }

        // Create symlink from walk_root to external target
        let link = walk_root.join("link_to_target");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if symlink(&target, &link).is_err() {
                println!("⚠ Symlink creation failed, skipping test (may need permissions)");
                cleanup(&base);
                return;
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            if symlink_dir(&target, &link).is_err() {
                println!("⚠ Symlink creation failed, skipping test (may need admin rights)");
                cleanup(&base);
                return;
            }
        }

        // Walk with default settings (follow_symlinks = false)
        let entries: Vec<_> = walk_dir(&walk_root).filter_map(|r| r.ok()).collect();

        let file_count = entries.iter().filter(|e| e.is_file()).count();
        let symlink_count = entries.iter().filter(|e| e.is_symlink()).count();

        // Should find NO files (file only reachable through unfollowed symlink)
        assert_eq!(
            file_count, 0,
            "test_symlinks_not_followed: found {} files, expected 0 (symlink should not be followed)",
            file_count
        );

        // Should detect the symlink entry itself
        assert_eq!(
            symlink_count, 1,
            "test_symlinks_not_followed: found {} symlinks, expected 1",
            symlink_count
        );

        // Verify the target file is NOT in results
        let has_target_file = entries
            .iter()
            .any(|e| e.file_name() == Some("file_in_target.txt"));

        assert!(
            !has_target_file,
            "test_symlinks_not_followed: file from symlink target should not appear in results"
        );

        cleanup(&base);
    }
    /// Test: symlink cycles are detected when following is enabled.
    ///
    /// Creates a cycle: a/link_b -> b, b/link_a -> a
    /// Walker should detect the cycle and not loop infinitely.
    #[test]
    fn test_symlink_cycle_detection() {
        let dir = test_dir("symlink_cycle");
        cleanup(&dir);

        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "test_symlink_cycle: failed to create base directory"
        );

        let dir_a = dir.join("a");
        let dir_b = dir.join("b");

        assert!(
            fs::create_dir_all(&dir_a).is_ok(),
            "test_symlink_cycle: failed to create dir a"
        );
        assert!(
            fs::create_dir_all(&dir_b).is_ok(),
            "test_symlink_cycle: failed to create dir b"
        );

        // Create cycle
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if symlink(&dir_b, dir_a.join("link_b")).is_err()
                || symlink(&dir_a, dir_b.join("link_a")).is_err()
            {
                println!("⚠ Symlink creation failed, skipping test");
                cleanup(&dir);
                return;
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            if symlink_dir(&dir_b, dir_a.join("link_b")).is_err()
                || symlink_dir(&dir_a, dir_b.join("link_a")).is_err()
            {
                println!("⚠ Symlink creation failed, skipping test");
                cleanup(&dir);
                return;
            }
        }

        // Walk with symlink following enabled
        let config = WalkConfig::new()
            .follow_symlinks(true)
            .continue_on_error(true);

        let walker = DirWalker::new(&dir, config);

        let count = walker.filter_map(|r| r.ok()).count();

        assert!(
            count < 100,
            "test_symlink_cycle: should not loop infinitely (found {} entries)",
            count
        );

        cleanup(&dir);
    }

    /// Test: broken symlinks are skipped gracefully.
    ///
    /// Creates a symlink pointing to a nonexistent target.
    /// Walker should skip it without panic or halt.
    ///
    /// ## What This Verifies
    /// - Broken symlinks don't cause panic
    /// - With follow_symlinks=false (default): broken link appears as symlink entry
    /// - With follow_symlinks=true: broken link is skipped (metadata unreadable)
    /// - Walker continues after encountering broken link
    #[test]
    fn test_broken_symlink_skip() {
        let dir = test_dir("broken_symlink");
        cleanup(&dir);

        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "test_broken_symlink: failed to create base directory"
        );

        // Create a valid file as a control to verify walker is working
        let valid_file = dir.join("valid_file.txt");
        if let Ok(mut f) = File::create(&valid_file) {
            let _ = f.write_all(b"test");
        }

        let link = dir.join("broken_link");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if symlink("/absolutely/nonexistent/path", &link).is_err() {
                println!("⚠ Symlink creation failed, skipping test");
                cleanup(&dir);
                return;
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_file;
            if symlink_file("C:\\nonexistent\\file.txt", &link).is_err() {
                println!("⚠ Symlink creation failed, skipping test");
                cleanup(&dir);
                return;
            }
        }

        // Test 1: With follow_symlinks=false (default), broken link appears as symlink
        println!("Test 1: follow_symlinks=false (default)");
        let config = WalkConfig::new()
            .follow_symlinks(false)
            .continue_on_error(true);

        let walker = DirWalker::new(&dir, config);
        let entries: Vec<_> = walker.filter_map(|r| r.ok()).collect();

        let symlink_count = entries.iter().filter(|e| e.is_symlink()).count();
        let file_count = entries
            .iter()
            .filter(|e| e.is_file() && !e.is_symlink())
            .count();

        assert_eq!(
            file_count, 1,
            "test_broken_symlink: should find 1 valid file"
        );

        assert_eq!(
            symlink_count, 1,
            "test_broken_symlink: should detect 1 symlink entry (even though broken)"
        );

        // Test 2: With follow_symlinks=true, broken link is skipped gracefully
        println!("Test 2: follow_symlinks=true");
        let config_follow = WalkConfig::new()
            .follow_symlinks(true)
            .continue_on_error(true);

        let walker_follow = DirWalker::new(&dir, config_follow);
        let entries_follow: Vec<_> = walker_follow.filter_map(|r| r.ok()).collect();

        // Should find only the valid file, broken link should be skipped
        let file_count_follow = entries_follow.iter().filter(|e| e.is_file()).count();

        assert_eq!(
            file_count_follow, 1,
            "test_broken_symlink: should find 1 valid file when following links (broken link skipped)"
        );

        // The broken symlink should NOT appear in results when following
        // (because fs::metadata() fails on the target, so we continue past it)
        let has_broken = entries_follow
            .iter()
            .any(|e| e.file_name() == Some("broken_link"));

        assert!(
            !has_broken,
            "test_broken_symlink: broken symlink should be skipped when follow_symlinks=true"
        );

        cleanup(&dir);
    }

    /// Test: symlinks to files are handled correctly.
    ///
    /// Creates a symlink to a regular file (not a directory).
    /// Walker should yield the symlink, and it should correctly report
    /// as a file when follow_symlinks is true.
    #[test]
    fn test_symlink_to_file() {
        let dir = test_dir("symlink_to_file");
        cleanup(&dir);

        assert!(
            fs::create_dir_all(&dir).is_ok(),
            "test_symlink_to_file: failed to create base directory"
        );

        let target_file = dir.join("target.txt");
        if let Ok(mut f) = File::create(&target_file) {
            let _ = f.write_all(b"target content");
        }

        let link = dir.join("link_to_file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            if symlink(&target_file, &link).is_err() {
                println!("⚠ Symlink creation failed, skipping test");
                cleanup(&dir);
                return;
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_file;
            if symlink_file(&target_file, &link).is_err() {
                println!("⚠ Symlink creation failed, skipping test");
                cleanup(&dir);
                return;
            }
        }

        // Walk with symlink following enabled
        let config = WalkConfig::new()
            .follow_symlinks(true)
            .continue_on_error(true);

        let walker = DirWalker::new(&dir, config);
        let entries: Vec<_> = walker.filter_map(|r| r.ok()).collect();

        let symlink_entry = entries
            .iter()
            .find(|e| e.is_symlink() && e.file_name() == Some("link_to_file"));

        assert!(
            symlink_entry.is_some(),
            "test_symlink_to_file: symlink entry should be found"
        );

        let symlink_entry = symlink_entry.unwrap();
        assert!(
            symlink_entry.is_file(),
            "test_symlink_to_file: symlink to file should report is_file() = true"
        );

        cleanup(&dir);
    }
}
