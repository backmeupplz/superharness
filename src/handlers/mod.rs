pub mod checkpoint_cmds;
pub mod diagnostic_cmds;
pub mod display_cmds;
pub mod git_cmds;
pub mod harness_cmds;
pub mod heartbeat_cmds;
pub mod init;
pub mod pane_cmds;
pub mod pending_cmds;
pub mod spawn;
pub mod status;

// Re-export all handler functions for convenient access from main.rs.
pub use checkpoint_cmds::{handle_checkpoint, handle_checkpoints, handle_memory, handle_resume};
pub use diagnostic_cmds::{handle_ask, handle_healthcheck, handle_respawn};
pub use display_cmds::{handle_event_feed, handle_tasks_modal};
pub use git_cmds::handle_git_check;
pub use harness_cmds::{
    handle_harness_list, handle_harness_set, handle_harness_settings, handle_harness_switch,
};
pub use heartbeat_cmds::{handle_heartbeat, handle_heartbeat_status, handle_heartbeat_toggle};
pub use init::handle_init;
pub use pane_cmds::{
    handle_compact, handle_hide, handle_kill, handle_layout, handle_list, handle_read,
    handle_resize, handle_send, handle_show, handle_smart_layout, handle_surface,
};
pub use pending_cmds::{handle_run_pending, handle_tasks};
pub use spawn::handle_spawn;
pub use status::{
    handle_status_counts, handle_status_human, handle_terminal_size, handle_toggle_mode,
    handle_workers,
};
