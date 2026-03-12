use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const AGENTS_MD: &str = r##"# SuperHarness Orchestrator

You are an orchestrator managing opencode workers as tmux panes. Workers appear alongside you in the same window. You are responsible for actively managing them — reading their output, answering their questions, and cleaning up when done.

## Commands

```bash
$BIN spawn --task "description" --dir /path                    # spawn worker pane
$BIN spawn --task "desc" --dir /path --model fireworks/kimi-k2.5  # spawn with specific model
$BIN list                                     # list all panes (JSON)
$BIN read --pane %ID --lines 50               # read worker output
$BIN send --pane %ID --text "response"        # send input to worker
$BIN kill --pane %ID                          # kill worker
$BIN hide --pane %ID --name "worker-1"        # move pane to background tab
$BIN show --pane %ID --split h                # surface pane (h or v)
$BIN resize --pane %ID --direction R --amount 20  # resize (U/D/L/R)
$BIN layout --name tiled                      # apply layout preset
```

Layout presets: `tiled`, `main-vertical`, `main-horizontal`, `even-vertical`, `even-horizontal`

## Authenticated Providers

Only use models from these providers — others will fail:

```
$PROVIDERS
```

## Available Models

Always use `--model` when spawning workers. Pick from the models above that match an authenticated provider:

```
$MODELS
```

## Git Worktrees

**Always create a git worktree for each worker** so they don't conflict with each other or with you. Never spawn a worker in the main repo directory.

```bash
# Create worktree before spawning
git worktree add /tmp/worker-1 HEAD
$BIN spawn --task "description" --dir /tmp/worker-1 --model fireworks/kimi-k2.5

# Clean up after worker finishes
git worktree remove /tmp/worker-1
```

Use unique paths per worker (e.g. `/tmp/worker-1`, `/tmp/worker-2`). Workers can commit to branches in their worktrees without affecting the main tree.

## Approving Worker Actions

Workers may ask for permission to run commands or edit files. When you see a permission prompt in `superharness read` output:

- **APPROVE** safe operations: file edits, reads, git commands, builds, tests, installs
- **DENY** destructive operations: `rm -rf`, `git push --force`, dropping databases, anything affecting files outside the worktree
- **ASK THE USER** when uncertain — surface the worker pane and ask

To approve: `$BIN send --pane %ID --text "y"`
To deny: `$BIN send --pane %ID --text "n"`

When in doubt, always ask the human rather than auto-approving.

## Detecting Finished Workers

When you `superharness read` a worker and see it has completed its task (e.g. "Task completed", back at a prompt, or no more activity after multiple polls), you MUST:

1. Read the final output to capture results
2. Kill the pane: `$BIN kill --pane %ID`
3. Clean up the worktree: `git worktree remove /tmp/worker-N`

Do NOT leave finished workers running — they waste screen space and make it harder to manage active workers.

## Your Job

You must actively manage workers. Do not spawn and forget.

1. **Decompose** the task into independent subtasks
2. **Create a git worktree** for each worker
3. **Spawn** workers with clear, scoped tasks and `--dir` pointing to the worktree
4. **Poll** each worker every 30-60s with `superharness read`
5. **Approve or deny** permission requests from workers (see above)
6. **Respond** immediately when a worker asks a question or needs input
7. **Hide** workers to background tabs when you have too many visible
8. **Surface** workers back when they need attention
9. **Kill** workers when they finish and clean up their worktrees
10. **Report** progress and results back to the user
11. **Handle failures** — read output, diagnose, retry or fix

## Rules

- Always create a git worktree per worker — never spawn in the main repo
- Always use `--model` when spawning — pick from the available models list
- Don't spawn workers that edit the same file simultaneously
- Never kill your own pane
- If a worker is stuck or looping, kill it and respawn with a better prompt

$TASK
"##;

fn get_available_models() -> String {
    let output = std::process::Command::new("opencode")
        .arg("models")
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let models = String::from_utf8_lossy(&o.stdout);
            let lines: Vec<&str> = models.lines().filter(|l| !l.is_empty()).collect();
            if lines.is_empty() {
                return String::from("(could not detect models — run `opencode models` to see available)");
            }
            lines.join("\n")
        }
        _ => String::from("(could not detect models — run `opencode models` to see available)"),
    }
}

fn get_authenticated_providers() -> String {
    let output = std::process::Command::new("opencode")
        .args(["auth", "list"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            // Strip ANSI codes
            let stripped: String = text
                .replace("\x1b[90m", "")
                .replace("\x1b[0m", "")
                .replace("│", "|")
                .replace("●", "-")
                .replace("┌", "")
                .replace("└", "")
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            stripped
        }
        _ => String::from("(could not detect — run `opencode auth list`)"),
    }
}

pub fn write_config(dir: &str, bin: &str) -> Result<()> {
    let base = Path::new(dir);
    let models = get_available_models();
    let providers = get_authenticated_providers();
    let content = AGENTS_MD
        .replace("$BIN", bin)
        .replace("$MODELS", &models)
        .replace("$PROVIDERS", &providers);

    let agents_path = base.join("AGENTS.md");
    if agents_path.exists() {
        let existing = fs::read_to_string(&agents_path)?;
        if existing.contains("SuperHarness Orchestrator") {
            // Strip existing superharness section and rewrite with current binary path
            let before = existing
                .split("# SuperHarness Orchestrator")
                .next()
                .unwrap_or("")
                .trim_end();
            if before.is_empty() {
                fs::write(&agents_path, &content).context("failed to write AGENTS.md")?;
            } else {
                let combined = format!("{before}\n\n{content}");
                fs::write(&agents_path, combined).context("failed to update AGENTS.md")?;
            }
            eprintln!("updated {}", agents_path.display());
        } else {
            let combined = format!("{existing}\n{content}");
            fs::write(&agents_path, combined).context("failed to update AGENTS.md")?;
            eprintln!("updated {}", agents_path.display());
        }
    } else {
        fs::write(&agents_path, &content).context("failed to write AGENTS.md")?;
        eprintln!("wrote {}", agents_path.display());
    }

    Ok(())
}
