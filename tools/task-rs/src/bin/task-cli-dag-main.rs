
// ============================================================================
// MAIN
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = open_db()?;

    match cli.command {
        Commands::Add { content, priority, depends_on } => {
            let id = add_task(&conn, &content, &priority, depends_on.as_deref())?;
            let priority_enum = TaskPriority::from_str(&priority);
            println!("Task added: #{}", id);
            if priority_enum != TaskPriority::Normal {
                println!("Priority: {}", priority_enum);
            }
            if let Some(deps) = depends_on {
                println!("Depends on: {}", deps);
            }
        }

        Commands::List { all, completed, limit } => {
            let tasks = list_tasks(&conn, all, completed, limit)?;

            if tasks.is_empty() {
                println!("No tasks");
            } else {
                // Group by status
                let in_progress: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::InProgress).collect();
                let ready: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Ready).collect();
                let blocked: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Blocked).collect();
                let pending: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Pending).collect();
                let val_failed: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::ValidationFailed).collect();
                let executed: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Executed).collect();
                let validated: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Validated).collect();

                let active_count = in_progress.len() + ready.len() + blocked.len() + pending.len() + val_failed.len();

                if active_count > 0 {
                    println!("=== ACTIVE TASKS ({}) ===", active_count);
                    for task in &in_progress { print_task_line(task); }
                    for task in &ready { print_task_line(task); }
                    for task in &blocked { print_task_line(task); }
                    for task in &val_failed { print_task_line(task); }
                    for task in &pending { print_task_line(task); }
                }

                if (!executed.is_empty() || !validated.is_empty()) && (all || completed) {
                    if active_count > 0 { println!(); }
                    println!("=== COMPLETED ({}) ===", executed.len() + validated.len());
                    for task in &executed { print_task_line(task); }
                    for task in &validated { print_task_line(task); }
                }
            }
        }

        Commands::Start { id } => {
            if update_status(&conn, id, TaskStatus::InProgress)? {
                println!("Task #{} started", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Execute { id, output } => {
            if execute_task(&conn, id, output.as_deref())? {
                println!("Task #{} executed (awaiting validation)", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Validate { id, pass, fail, reason } => {
            if pass == fail {
                println!("Error: specify either --pass or --fail");
                return Ok(());
            }

            let messages = validate_task(&conn, id, pass, reason.as_deref())?;
            for msg in messages {
                println!("{}", msg);
            }
        }

        Commands::Block { id, reason } => {
            if block_task(&conn, id, &reason)? {
                println!("Task #{} blocked: {}", id, reason);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Unblock { id } => {
            if update_status(&conn, id, TaskStatus::Pending)? {
                println!("Task #{} unblocked", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Depend { id, on } => {
            add_dependency(&conn, id, on)?;
            println!("Task #{} now depends on #{}", id, on);
        }

        Commands::Undepend { id, from } => {
            remove_dependency(&conn, id, from)?;
            println!("Removed dependency: #{} no longer depends on #{}", id, from);
        }

        Commands::Ready => {
            let ready = get_ready_tasks(&conn)?;
            if ready.is_empty() {
                println!("No tasks ready to execute");
            } else {
                println!("=== READY TO EXECUTE ({}) ===", ready.len());
                for task in &ready {
                    print_task_line(task);
                }
            }
        }

        Commands::Parallel => {
            let parallel = get_parallel_tasks(&conn)?;
            if parallel.is_empty() {
                println!("No tasks can run in parallel");
            } else {
                println!("=== CAN RUN IN PARALLEL ({}) ===", parallel.len());
                for task in &parallel {
                    print_task_line(task);
                }
            }
        }

        Commands::CriticalPath => {
            let critical = get_critical_path(&conn)?;
            if critical.is_empty() {
                println!("No critical path (no dependencies)");
            } else {
                println!("=== CRITICAL PATH ({} tasks) ===", critical.len());
                for (i, task) in critical.iter().enumerate() {
                    let prefix = if i > 0 { " -> " } else { "    " };
                    println!("{}{} #{} {}", prefix, task.status.symbol(), task.id, truncate(&task.content, 50));
                }
            }
        }

        Commands::Dag => {
            let viz = visualize_dag(&conn)?;
            println!("{}", viz);
        }

        Commands::Delete { id } => {
            if delete_task(&conn, id)? {
                println!("Task #{} deleted", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Get { id } => {
            match load_task(&conn, id)? {
                Some(task) => {
                    println!("Task #{}", task.id);
                    println!("Status: {}", task.status);
                    println!("Priority: {}", task.priority);
                    println!("Created: {}", format_relative_time(&task.created));
                    println!("Updated: {}", format_relative_time(&task.updated));
                    if !task.depends_on.is_empty() {
                        println!("Depends on: {:?}", task.depends_on);
                    }
                    if let Some(reason) = &task.blocked_reason {
                        println!("Blocked: {}", reason);
                    }
                    if let Some(reason) = &task.validation_reason {
                        println!("Validation: {}", reason);
                    }
                    if let Some(output) = &task.output {
                        println!("Output: {}", output);
                    }
                    println!();
                    println!("{}", task.content);
                }
                None => println!("Task #{} not found", id),
            }
        }

        Commands::Stats => {
            let stats = get_stats(&conn)?;
            let ai_id = get_ai_id();
            let total = stats.get("total").unwrap_or(&0);
            let deps = stats.get("dependencies").unwrap_or(&0);

            println!("Task DAG Statistics (AI: {}):", ai_id);
            println!("  Total tasks: {}", total);
            println!("  Dependencies: {}", deps);
            println!();
            println!("  Active:");
            println!("    [ ] Pending: {}", stats.get("pending").unwrap_or(&0));
            println!("    [R] Ready: {}", stats.get("ready").unwrap_or(&0));
            println!("    [>] In progress: {}", stats.get("in_progress").unwrap_or(&0));
            println!("    [!] Blocked: {}", stats.get("blocked").unwrap_or(&0));
            println!("    [X] Validation failed: {}", stats.get("validation_failed").unwrap_or(&0));
            println!();
            println!("  Done:");
            println!("    [*] Executed: {}", stats.get("executed").unwrap_or(&0));
            println!("    [V] Validated: {}", stats.get("validated").unwrap_or(&0));
        }
    }

    Ok(())
}
