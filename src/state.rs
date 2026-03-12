use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Present,
    Away,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Present => write!(f, "present"),
            Mode::Away => write!(f, "away"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDecision {
    pub id: String,
    pub pane: String,
    pub question: String,
    pub asked_at: u64,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub mode: Mode,
    pub away_since: Option<u64>,
    pub away_message: Option<String>,
    pub pending_decisions: Vec<PendingDecision>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::Present,
            away_since: None,
            away_message: None,
            pending_decisions: Vec::new(),
        }
    }
}

pub struct StateManager {
    path: PathBuf,
}

impl StateManager {
    pub fn new() -> Result<Self> {
        let data_dir = dirs_state_path()?;
        fs::create_dir_all(&data_dir)
            .with_context(|| format!("failed to create state directory: {}", data_dir.display()))?;
        let path = data_dir.join("state.json");
        Ok(Self { path })
    }

    fn load(&self) -> Result<State> {
        if !self.path.exists() {
            return Ok(State::default());
        }
        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read state file: {}", self.path.display()))?;
        let state: State = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse state file: {}", self.path.display()))?;
        Ok(state)
    }

    fn save(&self, state: &State) -> Result<()> {
        let content = serde_json::to_string_pretty(state).context("failed to serialize state")?;
        fs::write(&self.path, content)
            .with_context(|| format!("failed to write state file: {}", self.path.display()))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_mode(&self) -> Result<Mode> {
        Ok(self.load()?.mode)
    }

    pub fn set_mode(&self, mode: Mode, message: Option<&str>) -> Result<()> {
        let mut state = self.load()?;
        match &mode {
            Mode::Away => {
                state.away_since = Some(now_unix());
                state.away_message = message.map(|s| s.to_string());
            }
            Mode::Present => {
                state.away_since = None;
                state.away_message = None;
            }
        }
        state.mode = mode;
        self.save(&state)
    }

    pub fn add_pending_decision(
        &self,
        pane: &str,
        question: &str,
        context: &str,
    ) -> Result<String> {
        let mut state = self.load()?;
        let id = format!("{}", now_unix());
        let decision = PendingDecision {
            id: id.clone(),
            pane: pane.to_string(),
            question: question.to_string(),
            asked_at: now_unix(),
            context: context.to_string(),
        };
        state.pending_decisions.push(decision);
        self.save(&state)?;
        Ok(id)
    }

    pub fn get_pending_decisions(&self) -> Result<Vec<PendingDecision>> {
        Ok(self.load()?.pending_decisions)
    }

    pub fn clear_decisions(&self) -> Result<()> {
        let mut state = self.load()?;
        state.pending_decisions.clear();
        self.save(&state)
    }

    #[allow(dead_code)]
    pub fn is_away(&self) -> bool {
        self.load().map(|s| s.mode == Mode::Away).unwrap_or(false)
    }

    pub fn get_state(&self) -> Result<State> {
        self.load()
    }
}

fn dirs_state_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("cannot determine home directory (HOME not set)")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
