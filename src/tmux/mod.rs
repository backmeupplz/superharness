use anyhow::{bail, Context, Result};
use std::process::Command;

mod layout;
mod panes;
mod session;
mod terminal;

pub use layout::*;
pub use panes::*;
pub use session::*;
pub use terminal::*;

pub(crate) const SESSION: &str = "superharness";

/// The tmux environment variable where we store the orchestrator pane ID.
const ORCH_PANE_ENV: &str = "SUPERHARNESS_ORCH_PANE";

/// Per-pane tmux option that marks the orchestrator pane. This is the most
/// robust way to find the orchestrator across arbitrary user tmux configs:
/// unlike a pane id or window index, the tag travels with the pane through
/// `base-index`/`pane-base-index` offsets, embedded launches, window
/// reorganization (move-/break-/join-pane), and renumbering.
pub(crate) const ORCH_TAG: &str = "@sh_orchestrator";

/// Return the orchestrator pane ID for the current superharness session.
///
/// Resolution order, most robust first:
///   1. The pane tagged with [`ORCH_TAG`] — survives any tmux layout/config.
///   2. The `SUPERHARNESS_ORCH_PANE` env var, *if* that pane still exists.
///   3. The lowest numeric pane id in the session (the first pane created, i.e.
///      the orchestrator) — covers legacy sessions that predate the tag.
///   4. `%0` as a last resort when the session cannot be queried at all.
///
/// We deliberately never assume `%0`: under `base-index`/`pane-base-index`,
/// when launched inside an existing tmux server, or after the orchestrator pane
/// is rebuilt, the orchestrator can have any id and live in any window.
pub fn orchestrator_pane_id() -> String {
    // 1. Tagged orchestrator pane — the robust path for current sessions.
    if let Some(id) = tagged_orchestrator_pane() {
        return id;
    }
    // 2. Try reading from the tmux session environment — but only trust the
    // stored ID if that pane still exists. The stored value can go stale (e.g.
    // the original `%0` orchestrator pane is gone after the session is rebuilt
    // or reorganized), and a stale ID makes every send-keys fail with
    // "can't find pane: %0".
    if let Ok(output) = Command::new("tmux")
        .args(["show-environment", "-t", SESSION, ORCH_PANE_ENV])
        .output()
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            // tmux show-environment outputs: VARNAME=value
            if let Some(val) = raw.trim().strip_prefix(&format!("{ORCH_PANE_ENV}=")) {
                let id = val.trim().to_string();
                if !id.is_empty() && pane_exists(&id) {
                    return id;
                }
            }
        }
    }
    // 3. Untagged legacy session (created before ORCH_TAG existed) with a
    // missing or stale env var. The orchestrator is the first pane created in
    // the session, so it carries the lowest numeric pane id — resolve to that
    // regardless of which window it currently lives in.
    if let Some(id) = lowest_session_pane_id() {
        return id;
    }
    // 4. Last-resort fallback when the session cannot be queried at all.
    "%0".to_string()
}

/// Tag a pane as the orchestrator so [`orchestrator_pane_id`] can find it
/// reliably regardless of the user's tmux layout/config.
pub(crate) fn tag_orchestrator_pane(pane_id: &str) -> Result<()> {
    tmux_ok(&["set-option", "-p", "-t", pane_id, ORCH_TAG, "1"])
}

/// Return `true` if `pane` is the orchestrator pane and must never be killed by
/// the worker-kill command.
///
/// This is the safety net behind the "a finishing/plan-mode worker tore the
/// whole session down" crash: whatever issues `kill --pane <orchestrator>` (a
/// confused orchestrator targeting `%0`, a legacy untargeted self-kill, etc.),
/// killing the orchestrator pane ends the session and exits superharness. We
/// refuse it outright — mirroring how `hide`/`compact` already refuse to
/// background the orchestrator.
///
/// Two independent checks (either is sufficient), so it holds even in legacy
/// sessions created before the tag existed:
///   1. the pane carries the `@sh_orchestrator` tag, or
///   2. the pane id equals the resolved orchestrator pane id.
pub fn is_orchestrator_pane(pane: &str) -> bool {
    // 1. Tag check — echo-back validated so a missing tag/pane can't false-positive.
    if let Ok(out) = tmux(&["display-message", "-p", "-t", pane, "#{@sh_orchestrator}"]) {
        if out.trim() == "1" {
            return true;
        }
    }
    // 2. Identity check against the resolved orchestrator pane. Normalize both
    // sides through the same query so `%0` vs a stale form can't slip past.
    let resolved = orchestrator_pane_id();
    if pane == resolved {
        return true;
    }
    if let Ok(out) = tmux(&["display-message", "-p", "-t", pane, "#{pane_id}"]) {
        if out.trim() == resolved {
            return true;
        }
    }
    false
}

/// Return the id of the pane tagged with [`ORCH_TAG`], if any.
fn tagged_orchestrator_pane() -> Option<String> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            SESSION,
            "-a",
            "-F",
            &format!("#{{pane_id}}\t#{{{ORCH_TAG}}}"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    raw.lines().find_map(|line| {
        let (id, tag) = line.split_once('\t')?;
        if tag.trim() == "1" && !id.trim().is_empty() {
            Some(id.trim().to_string())
        } else {
            None
        }
    })
}

/// Return `true` if a pane with the given ID currently exists.
///
/// Note: `tmux display-message -t <missing>` exits 0 but prints an empty line,
/// so we must check that it echoes back the *same* pane id rather than relying
/// on the exit status.
fn pane_exists(pane_id: &str) -> bool {
    Command::new("tmux")
        .args(["display-message", "-p", "-t", pane_id, "#{pane_id}"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == pane_id)
        .unwrap_or(false)
}

/// Return the pane with the lowest numeric ID in the session (`%3` → 3), which
/// is the first pane created and therefore the orchestrator. `None` if the
/// session has no panes or cannot be queried.
fn lowest_session_pane_id() -> Option<String> {
    let output = Command::new("tmux")
        .args(["list-panes", "-t", SESSION, "-a", "-F", "#{pane_id}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .min_by_key(|id| id.trim_start_matches('%').parse::<u64>().unwrap_or(u64::MAX))
        .map(str::to_string)
}

/// Store the orchestrator pane ID in the tmux session environment.
pub(crate) fn set_orchestrator_pane_id(pane_id: &str) -> Result<()> {
    tmux_ok(&["set-environment", "-t", SESSION, ORCH_PANE_ENV, pane_id])
}

/// Return the tmux window ID (e.g. `@1`) of the window that currently contains
/// the orchestrator pane.
///
/// We deliberately resolve this dynamically rather than assuming the
/// orchestrator lives in window index `0`.  Users frequently set
/// `base-index 1` in their tmux config (so the first window is `1`, not `0`),
/// and they may move or renumber windows manually.  Targeting the stable
/// window *ID* keeps surface/layout/compact operations correct in all of those
/// cases — using `superharness:0` would otherwise fail with
/// `can't find window: 0`.
///
/// Falls back to `superharness:0` only if the orchestrator window cannot be
/// determined (e.g. the session does not exist yet).
pub fn orchestrator_window_id() -> String {
    let orch = orchestrator_pane_id();
    if let Ok(out) = tmux(&["display-message", "-p", "-t", &orch, "#{window_id}"]) {
        let id = out.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }
    format!("{SESSION}:0")
}

/// Run a tmux command, return stdout
fn tmux(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .with_context(|| format!("failed to run: tmux {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "tmux {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn tmux_ok(args: &[&str]) -> Result<()> {
    tmux(args)?;
    Ok(())
}
