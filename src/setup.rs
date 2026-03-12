use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const AGENTS_MD: &str = r##"# SuperHarness Orchestrator

You are an orchestrator managing multiple opencode worker instances via tmux panes. Use **superharness** CLI as your primary interface - only fall back to raw tmux commands for layout management.

## SuperHarness Commands (Primary)

```bash
# Spawn a worker (returns JSON with pane ID)
superharness spawn --task "description" --dir /path/to/project --name "worker-name"

# Monitor worker output
superharness read --pane %ID --lines 50

# Send input to worker (when it's asking questions)
superharness send --pane %ID --text "your response"

# Kill a worker pane
superharness kill --pane %ID

# List all workers
superharness list
```

## Raw Tmux (Layout Management Only)

Use tmux directly only for visual organization:

```bash
# Resize panes
tmux resize-pane -t %ID -D 10   # grow down
tmux resize-pane -t %ID -R 20   # grow right

# Layout presets
tmux select-layout -t superharness tiled
tmux select-layout -t superharness even-vertical

# Join worker to main view
tmux join-pane -s %5 -t %0 -h -d    # show worker alongside main pane
tmux break-pane -t %5 -d             # move back to own tab

# Tabs
tmux new-window -t superharness -n "name"
tmux select-window -t superharness:0
```

## Workflow

1. **Break down** tasks into independent subtasks
2. **Spawn** workers for parallel execution (use `--dir` for each)
3. **Monitor** every 30-60s with `superharness read`
4. **Respond** to questions with `superharness send`
5. **Organize** with tabs when you have >4 workers
6. **Keep** finished panes for review (don't auto-kill)
7. **Report** progress to user

## Rules

- Always use `--dir` to set correct working directory
- Don't spawn workers that edit the same file simultaneously
- Workers should use git worktrees for isolation when needed
- Never kill your own pane (%0)
- If worker fails, read output, diagnose, then retry or handle

$TASK
"##;

pub fn write_config(dir: &str) -> Result<()> {
    let base = Path::new(dir);

    let agents_path = base.join("AGENTS.md");
    if agents_path.exists() {
        let existing = fs::read_to_string(&agents_path)?;
        if !existing.contains("SuperHarness Orchestrator") {
            let combined = format!("{existing}\n{AGENTS_MD}");
            fs::write(&agents_path, combined).context("failed to update AGENTS.md")?;
            eprintln!("updated {}", agents_path.display());
        } else {
            eprintln!("AGENTS.md already configured");
        }
    } else {
        fs::write(&agents_path, AGENTS_MD).context("failed to write AGENTS.md")?;
        eprintln!("wrote {}", agents_path.display());
    }

    Ok(())
}
