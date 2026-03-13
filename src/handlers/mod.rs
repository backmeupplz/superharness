pub mod git_cmds;
pub mod heartbeat_cmds;
pub mod init;
pub mod misc;
pub mod relay_cmds;
pub mod spawn;
pub mod status;
pub mod task_cmds;

// Re-export all handler functions for convenient access from main.rs.
pub use git_cmds::handle_git_check;
pub use heartbeat_cmds::{handle_heartbeat, handle_heartbeat_status, handle_heartbeat_toggle};
pub use init::handle_init;
pub use misc::{
    handle_ask, handle_checkpoint, handle_checkpoints, handle_compact, handle_event_feed,
    handle_harness_list, handle_harness_set, handle_harness_settings, handle_harness_switch,
    handle_healthcheck, handle_hide, handle_kill, handle_layout, handle_list, handle_loop_clear,
    handle_loop_status, handle_memory, handle_read, handle_resize, handle_respawn, handle_resume,
    handle_run_pending, handle_send, handle_show, handle_smart_layout, handle_surface,
    handle_tasks, handle_tasks_modal,
};
pub use relay_cmds::{
    handle_relay, handle_relay_answer, handle_relay_list, handle_sudo_exec, handle_sudo_relay,
};
pub use spawn::handle_spawn;
pub use status::{
    handle_status_counts, handle_status_human, handle_terminal_size, handle_toggle_mode,
    handle_workers,
};
pub use task_cmds::{
    handle_subtask_add, handle_subtask_done, handle_task_add, handle_task_block,
    handle_task_cancel, handle_task_done, handle_task_list, handle_task_remove, handle_task_show,
    handle_task_start,
};
