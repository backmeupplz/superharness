//! Shared utility functions used across superharness modules.
//!
//! Centralises duplicated helpers so every module imports from one place
//! instead of each defining its own copy.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// Returns the current Unix timestamp in whole seconds.
/// Returns 0 on error (e.g. time went backwards).
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Hash an arbitrary string to a `u64` using the stdlib `DefaultHasher`.
///
/// This is NOT cryptographically secure — it is only used for change
/// detection and loop-guard content fingerprinting.
pub fn hash_string(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// ID generation
// ---------------------------------------------------------------------------

/// Generate a unique ID with the given prefix.
///
/// Format: `<prefix>-<timestamp_hex><random_hex>`
///
/// Uses the current Unix timestamp (hex) plus 4 random bytes read from
/// `/dev/urandom` (hex) for a compact, reasonably unique identifier.
pub fn generate_id(prefix: &str) -> String {
    let ts = now_unix();
    let rand_hex = read_random_hex4();
    format!("{prefix}-{ts:x}{rand_hex}")
}

/// Read 4 bytes from `/dev/urandom` and format them as 8 hex chars.
/// Falls back to subsecond nanoseconds if `/dev/urandom` is unavailable
/// (e.g. non-Unix systems or unusual sandboxes).
fn read_random_hex4() -> String {
    let mut buf = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() {
            return buf.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    // Fallback: use subsecond nanos as a cheap pseudo-random source
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{nanos:08x}")
}

// ---------------------------------------------------------------------------
// Shell escaping
// ---------------------------------------------------------------------------

/// Minimal POSIX shell escaping: wrap the value in single quotes and
/// escape any existing single-quote characters with `'\''`.
pub fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Config directory
// ---------------------------------------------------------------------------

/// Return the superharness config directory.
///
/// Resolves `~/.config/superharness` (or the platform-appropriate config
/// base via `dirs::config_dir`).  Falls back to `~/.config` if `dirs`
/// cannot determine the config directory.
pub fn superharness_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("superharness")
}

// ---------------------------------------------------------------------------
// Data directory
// ---------------------------------------------------------------------------

/// Return the superharness data directory (`~/.local/share/superharness/`).
/// Falls back to `/tmp/superharness` if HOME cannot be determined.
pub fn superharness_data_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
}

// ---------------------------------------------------------------------------
// ANSI color / style constants
// ---------------------------------------------------------------------------

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const UNDERLINE: &str = "\x1b[4m";
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
pub const BRIGHT_RED: &str = "\x1b[91m";
