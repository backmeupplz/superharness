use anyhow::Result;

use crate::relay;

/// Handle `Command::Relay`.
pub fn handle_relay(
    pane: String,
    question: String,
    context: String,
    sensitive: bool,
    wait_for: Option<String>,
    timeout: u64,
) -> Result<()> {
    // If --wait-for is given, poll for the answer to an existing request.
    if let Some(ref req_id) = wait_for {
        match relay::wait_for_answer(req_id, timeout)? {
            Some(answer) => {
                let out = serde_json::json!({
                    "id": req_id,
                    "answered": true,
                    "answer": if sensitive { "<redacted>" } else { &answer },
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                // Print the raw answer on its own line so workers can
                // capture it with $(...) command substitution.
                eprintln!("{answer}");
            }
            None => {
                let out = serde_json::json!({
                    "id": req_id,
                    "answered": false,
                    "note": "timeout expired — no answer received",
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                std::process::exit(1);
            }
        }
    } else {
        // Create a new relay request.
        let id = relay::add_relay_request(&pane, &question, &context, sensitive)?;
        let out = serde_json::json!({
            "id": id,
            "pane": pane,
            "question": question,
            "context": context,
            "sensitive": sensitive,
            "status": "pending",
            "note": format!(
                "Relay request created. Poll for answer with: superharness relay --pane {pane} --question '' --wait-for {id}"
            ),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    }

    Ok(())
}

/// Handle `Command::RelayAnswer`.
pub fn handle_relay_answer(id: String, answer: String) -> Result<()> {
    relay::answer_relay(&id, &answer)?;
    let out = serde_json::json!({
        "id": id,
        "answered": true,
        "note": "Answer stored. The worker will receive it on its next poll.",
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::RelayList`.
pub fn handle_relay_list(pending: bool) -> Result<()> {
    let requests = if pending {
        relay::get_pending_relays()?
    } else {
        relay::list_all()?
    };

    let items: Vec<serde_json::Value> = requests
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "pane": r.pane_id,
                "kind": r.kind.to_string(),
                "question": r.question,
                "context": r.context,
                "sensitive": r.sensitive,
                "status": r.status.to_string(),
                // Never expose sensitive answers in list output.
                "answer": if r.sensitive { r.answer.as_ref().map(|_| "<redacted>") } else { r.answer.as_deref() },
                "created_at": r.created_at,
                "answered_at": r.answered_at,
            })
        })
        .collect();

    if requests.is_empty() {
        println!("No relay requests found.");
    } else {
        // Human-readable summary first.
        let pending_count = requests
            .iter()
            .filter(|r| r.status == relay::RelayStatus::Pending)
            .count();
        println!(
            "Relay requests: {} total, {} pending",
            requests.len(),
            pending_count
        );
        println!();

        for r in &requests {
            let status_marker = match r.status {
                relay::RelayStatus::Pending => "[PENDING ]",
                relay::RelayStatus::Answered => "[answered]",
                relay::RelayStatus::Cancelled => "[canceld ]",
            };
            let sens = if r.sensitive { " [sensitive]" } else { "" };
            println!("{status_marker} {} (pane {}){}", r.id, r.pane_id, sens);
            println!("  Q: {}", r.question);
            if !r.context.is_empty() {
                println!("  Context: {}", r.context);
            }
            if r.status == relay::RelayStatus::Pending {
                println!(
                    "  Answer with: superharness relay-answer --id {} --answer \"<value>\"",
                    r.id
                );
            }
            println!();
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "requests": items }))?
        );
    }

    Ok(())
}

/// Handle `Command::SudoRelay`.
pub fn handle_sudo_relay(pane: String, command: String, execute: bool, timeout: u64) -> Result<()> {
    let relay_id = relay::relay_sudo(&pane, &command)?;
    if execute {
        println!("Relay request {relay_id} created. Waiting for human to provide sudo password...");
        match relay::wait_for_answer(&relay_id, timeout)? {
            Some(password) => {
                let status = relay::run_sudo_with_password(&command, &password)?;
                let out = serde_json::json!({
                    "relay_id": relay_id,
                    "command": command,
                    "exit_code": status.code(),
                    "success": status.success(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                if !status.success() {
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
            None => {
                let out = serde_json::json!({
                    "relay_id": relay_id,
                    "answered": false,
                    "note": "timeout expired — sudo password not provided",
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                std::process::exit(1);
            }
        }
    } else {
        let out = serde_json::json!({
            "relay_id": relay_id,
            "pane": pane,
            "command": command,
            "status": "pending",
            "note": format!(
                "Sudo relay created. Human should run: superharness relay-answer --id {relay_id} --answer \"<password>\""
            ),
            "poll_command": format!(
                "superharness relay --pane {pane} --question '' --wait-for {relay_id}"
            ),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    }

    Ok(())
}

/// Handle `Command::SudoExec`.
pub fn handle_sudo_exec(pane: String, command: String, wait: bool, timeout: u64) -> Result<()> {
    use relay::SudoExecResult;

    match relay::sudo_exec(&pane, &command)? {
        SudoExecResult::Success => {
            let out = serde_json::json!({
                "pane": pane,
                "command": command,
                "success": true,
                "method": "nopasswd",
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        SudoExecResult::RelayCreated(relay_id) => {
            if wait {
                println!(
                    "sudo requires a password. Relay request {relay_id} created. Waiting for human..."
                );
                match relay::wait_for_answer(&relay_id, timeout)? {
                    Some(password) => {
                        let status = relay::run_sudo_with_password(&command, &password)?;
                        let out = serde_json::json!({
                            "relay_id": relay_id,
                            "pane": pane,
                            "command": command,
                            "exit_code": status.code(),
                            "success": status.success(),
                            "method": "relay_password",
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                        if !status.success() {
                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                    None => {
                        let out = serde_json::json!({
                            "relay_id": relay_id,
                            "answered": false,
                            "note": "timeout expired — sudo password not provided",
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                        std::process::exit(1);
                    }
                }
            } else {
                let out = serde_json::json!({
                    "relay_id": relay_id,
                    "pane": pane,
                    "command": command,
                    "status": "awaiting_password",
                    "note": format!(
                        "sudo requires a password. Answer with: superharness relay-answer --id {relay_id} --answer \"<password>\""
                    ),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        }
        SudoExecResult::Failed(msg) => {
            let out = serde_json::json!({
                "pane": pane,
                "command": command,
                "success": false,
                "error": msg,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
            std::process::exit(1);
        }
    }

    Ok(())
}
