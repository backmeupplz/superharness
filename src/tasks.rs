//! Canonical task-status handling.
//!
//! The orchestrator writes `.superharness/tasks.json` freeform, so the `status`
//! strings drift between vocabularies — e.g. a worker writes `"completed"` where
//! superharness historically expected `"done"`, or `"in_progress"` (underscore)
//! vs `"in-progress"` (hyphen). Before this module, an unrecognized status made
//! a task count toward the total but appear in NO status group: the F5 task
//! modal showed `Tasks: 7` with every per-status count `0` and no task bodies —
//! the tasks were effectively invisible.
//!
//! Everything that interprets a task's status MUST route it through
//! [`TaskStatus::from_raw`] so vocabulary drift can never hide a task again, and
//! anything genuinely unrecognized falls into [`TaskStatus::Other`] which is
//! still displayed (never dropped).

/// A task status mapped to a stable, superharness-internal category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    InProgress,
    Pending,
    Blocked,
    Done,
    Cancelled,
    /// Any status string we don't recognize. Deliberately retained and always
    /// surfaced (with its raw text) rather than silently dropped.
    Other,
}

impl TaskStatus {
    /// The order statuses are listed/grouped in the F5 modal. `Other` is last so
    /// unrecognized statuses are surfaced after the known ones.
    pub const DISPLAY_ORDER: [TaskStatus; 6] = [
        TaskStatus::InProgress,
        TaskStatus::Pending,
        TaskStatus::Blocked,
        TaskStatus::Done,
        TaskStatus::Cancelled,
        TaskStatus::Other,
    ];

    /// Map a raw `status` string to a canonical category. Case-insensitive and
    /// separator-insensitive (`_`/space normalized to `-`), with synonyms folded
    /// in. Unknown values become [`TaskStatus::Other`].
    pub fn from_raw(raw: &str) -> TaskStatus {
        let normalized = raw.trim().to_lowercase().replace([' ', '_'], "-");
        match normalized.as_str() {
            "in-progress" | "inprogress" | "in-flight" | "active" | "running" | "wip"
            | "started" | "doing" | "ongoing" => TaskStatus::InProgress,
            "pending" | "todo" | "to-do" | "queued" | "not-started" | "backlog" | "new"
            | "open" | "ready" => TaskStatus::Pending,
            "blocked" | "waiting" | "on-hold" | "hold" | "paused" | "stuck" => TaskStatus::Blocked,
            "done" | "completed" | "complete" | "finished" | "closed" | "merged" | "resolved"
            | "fixed" | "shipped" => TaskStatus::Done,
            "cancelled" | "canceled" | "skipped" | "wont-fix" | "wontfix" | "abandoned"
            | "dropped" | "obsolete" => TaskStatus::Cancelled,
            "" => TaskStatus::Other,
            _ => TaskStatus::Other,
        }
    }

    /// `true` if this status means the task is finished/completed (counts toward
    /// the "completed" half of the F5 `tasks(done/total)` label).
    pub fn is_completed(self) -> bool {
        matches!(self, TaskStatus::Done)
    }

    /// Upper-case label used as the group header in the F5 modal.
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::InProgress => "IN-PROGRESS",
            TaskStatus::Pending => "PENDING",
            TaskStatus::Blocked => "BLOCKED",
            TaskStatus::Done => "DONE",
            TaskStatus::Cancelled => "CANCELLED",
            TaskStatus::Other => "OTHER",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaskStatus;

    #[test]
    fn folds_completed_synonym_to_done() {
        // The exact bug: workers write "completed", not "done".
        assert_eq!(TaskStatus::from_raw("completed"), TaskStatus::Done);
        assert!(TaskStatus::from_raw("completed").is_completed());
        assert_eq!(TaskStatus::from_raw("done"), TaskStatus::Done);
    }

    #[test]
    fn normalizes_case_and_separators() {
        assert_eq!(TaskStatus::from_raw("In_Progress"), TaskStatus::InProgress);
        assert_eq!(TaskStatus::from_raw(" IN PROGRESS "), TaskStatus::InProgress);
        assert_eq!(TaskStatus::from_raw("ToDo"), TaskStatus::Pending);
    }

    #[test]
    fn recognized_buckets() {
        assert_eq!(TaskStatus::from_raw("pending"), TaskStatus::Pending);
        assert_eq!(TaskStatus::from_raw("blocked"), TaskStatus::Blocked);
        assert_eq!(TaskStatus::from_raw("cancelled"), TaskStatus::Cancelled);
        assert_eq!(TaskStatus::from_raw("canceled"), TaskStatus::Cancelled);
    }

    #[test]
    fn unknown_status_is_other_not_dropped() {
        // The key invariant: anything unrecognized is Other (still displayed),
        // never silently lost.
        assert_eq!(TaskStatus::from_raw("frobnicating"), TaskStatus::Other);
        assert_eq!(TaskStatus::from_raw(""), TaskStatus::Other);
        assert!(!TaskStatus::from_raw("frobnicating").is_completed());
    }
}
