use anyhow::Result;

use crate::tasks;

/// Handle `Command::TaskAdd`.
pub fn handle_task_add(
    title: String,
    description: Option<String>,
    priority: Option<String>,
    tags: Option<String>,
) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let priority = priority.as_deref().map(tasks::parse_priority).transpose()?;
    let tag_list: Vec<String> = tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let task = tm.add_task(&title, description.as_deref(), priority, tag_list)?;
    let id_short: String = task.id.chars().take(8).collect();
    println!("Task created: {id_short}  \"{}\"", task.title);
    println!("Full ID: {}", task.id);
    println!();
    println!("Reference this task with any unique prefix of its ID (e.g. '{id_short}').");
    Ok(())
}

/// Handle `Command::TaskList`.
pub fn handle_task_list(status: Option<String>, tag: Option<String>) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let status_filter = status
        .as_deref()
        .map(tasks::parse_status)
        .transpose()?
        .map(|s| s.to_string());
    let task_list = tm.list_tasks(status_filter.as_deref(), tag.as_deref())?;
    tasks::print_task_list(&task_list);
    Ok(())
}

/// Handle `Command::TaskDone`.
pub fn handle_task_done(id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let task = tm.set_status(&id, tasks::TaskStatus::Done)?;
    let id_short: String = task.id.chars().take(8).collect();
    println!("Task {id_short} marked as done: \"{}\"", task.title);
    Ok(())
}

/// Handle `Command::TaskStart`.
pub fn handle_task_start(id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let task = tm.set_status(&id, tasks::TaskStatus::InProgress)?;
    let id_short: String = task.id.chars().take(8).collect();
    println!("Task {id_short} marked as in_progress: \"{}\"", task.title);
    Ok(())
}

/// Handle `Command::TaskBlock`.
pub fn handle_task_block(id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let task = tm.set_status(&id, tasks::TaskStatus::Blocked)?;
    let id_short: String = task.id.chars().take(8).collect();
    println!("Task {id_short} marked as blocked: \"{}\"", task.title);
    Ok(())
}

/// Handle `Command::TaskCancel`.
pub fn handle_task_cancel(id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let task = tm.set_status(&id, tasks::TaskStatus::Cancelled)?;
    let id_short: String = task.id.chars().take(8).collect();
    println!("Task {id_short} marked as cancelled: \"{}\"", task.title);
    Ok(())
}

/// Handle `Command::TaskRemove`.
pub fn handle_task_remove(id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    tm.remove_task(&id)?;
    println!("Task removed.");
    Ok(())
}

/// Handle `Command::TaskShow`.
pub fn handle_task_show(id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let task = tm.get_task(&id)?;
    tasks::print_task_detail(&task);
    Ok(())
}

/// Handle `Command::SubtaskAdd`.
pub fn handle_subtask_add(task_id: String, title: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let subtask = tm.add_subtask(&task_id, &title)?;
    let sub_id_short: String = subtask.id.chars().take(8).collect();
    println!("Subtask created: {sub_id_short}  \"{}\"", subtask.title);
    Ok(())
}

/// Handle `Command::SubtaskDone`.
pub fn handle_subtask_done(task_id: String, subtask_id: String) -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    tm.complete_subtask(&task_id, &subtask_id)?;
    println!("Subtask marked as done.");
    Ok(())
}

/// Handle `Command::TaskCleanup`.
pub fn handle_task_cleanup() -> Result<()> {
    let tm = tasks::TaskManager::new()?;
    let (removed, remaining) = tm.cleanup_completed()?;
    println!("Removed {removed} completed/cancelled tasks. {remaining} tasks remaining.");
    Ok(())
}
