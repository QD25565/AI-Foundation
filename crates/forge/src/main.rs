//! Forge - AI-Foundation CLI
//!
//! A model-agnostic CLI for running AI assistants with full tool support.

mod ui;
mod config;
mod llm;
mod tools;
mod hooks;

use std::io;
use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use config::ForgeConfig;
use llm::{LlmProvider, ChatMessage, GenerationParams, StreamChunk, ToolCall, ToolDefinition, create_provider};
use tools::{builtin_tools, execute_tool};
use hooks::{HookExecutor, HookContext};
use ui::App;

/// Forge - AI-Foundation CLI
#[derive(Parser, Debug)]
#[command(name = "forge")]
#[command(author = "AI-Foundation Team")]
#[command(version)]
#[command(about = "AI-Foundation CLI — model-agnostic AI assistant with tool support")]
#[command(long_about = None)]
struct Args {
    /// Initial prompt to start with
    #[arg(short, long)]
    prompt: Option<String>,

    /// Model alias to use (from config)
    #[arg(short, long)]
    model: Option<String>,

    /// Auto-approve all tool calls
    #[arg(long)]
    auto_approve: bool,

    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Run setup wizard
    #[arg(long)]
    setup: bool,

    /// Continue from last session
    #[arg(short = 'c', long)]
    r#continue: bool,

    /// Resume a specific session
    #[arg(long)]
    resume: Option<String>,

    /// List available models
    #[arg(long)]
    list_models: bool,

    /// Headless mode: process prompt and print JSON result to stdout (no TUI)
    #[arg(long)]
    headless: bool,

    /// System prompt for headless mode
    #[arg(long)]
    system: Option<String>,

    /// Max tokens to generate in headless mode
    #[arg(long)]
    max_tokens: Option<u32>,

    /// Temperature for generation (0.0-2.0)
    #[arg(long)]
    temperature: Option<f32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Load configuration
    let mut config = ForgeConfig::load()?;

    // Apply command line overrides
    if let Some(model) = args.model {
        config.active_model = model;
    }
    if args.auto_approve {
        config.auto_approve = true;
    }

    // Handle special modes
    if args.list_models {
        list_models(&config);
        return Ok(());
    }

    if args.setup {
        run_setup(&mut config).await?;
        return Ok(());
    }

    // Try to create LLM provider
    let provider: Option<Arc<dyn LlmProvider>> = {
        if let (Some(model_config), Some(provider_config)) =
            (config.active_model_config(), config.active_provider())
        {
            match create_provider(provider_config, model_config).await {
                Ok(p) => {
                    println!("Connected to {} via {}", model_config.name, provider_config.name);
                    Some(Arc::from(p))
                }
                Err(e) => {
                    eprintln!("Warning: Could not create provider: {}", e);
                    eprintln!("Running in offline mode (placeholder responses)");
                    None
                }
            }
        } else {
            eprintln!("No model/provider configured. Running in offline mode.");
            None
        }
    };

    // Headless mode: process prompt and print JSON result, no TUI
    if args.headless {
        let prompt = args.prompt.unwrap_or_else(|| {
            eprintln!("Error: --prompt is required in headless mode");
            std::process::exit(1);
        });

        let Some(provider) = provider else {
            let err = serde_json::json!({"error": "No LLM provider available"});
            println!("{}", err);
            std::process::exit(1);
        };

        let system = args.system.unwrap_or_else(|| {
            "You are a helpful AI assistant. Be concise and direct.".to_string()
        });

        let messages = vec![
            ChatMessage { role: llm::MessageRole::System, content: system, name: None, tool_call_id: None },
            ChatMessage { role: llm::MessageRole::User, content: prompt, name: None, tool_call_id: None },
        ];

        let params = GenerationParams {
            max_tokens: args.max_tokens.unwrap_or(512) as usize,
            temperature: args.temperature.unwrap_or(0.3),
            stop_sequences: vec![],
            tools: vec![],
        };

        let mut result_text = String::new();
        let mut stream = provider.generate_stream(&messages, &params).await
            .unwrap_or_else(|e| {
                let err = serde_json::json!({"error": format!("{}", e)});
                println!("{}", err);
                std::process::exit(1);
            });

        while let Some(chunk) = stream.next().await {
            match chunk {
                StreamChunk::Text(t) => result_text.push_str(&t),
                StreamChunk::Done { usage, .. } => {
                    let (prompt_tokens, completion_tokens) = usage
                        .map(|u| (u.prompt_tokens, u.completion_tokens))
                        .unwrap_or((0, 0));
                    let result = serde_json::json!({
                        "content": result_text,
                        "usage": {
                            "input_tokens": prompt_tokens,
                            "output_tokens": completion_tokens
                        }
                    });
                    println!("{}", result);
                    return Ok(());
                }
                StreamChunk::Error(e) => {
                    let err = serde_json::json!({"error": format!("{}", e)});
                    println!("{}", err);
                    std::process::exit(1);
                }
                _ => {}
            }
        }

        // Stream ended without Done chunk
        let result = serde_json::json!({
            "content": result_text,
            "usage": { "input_tokens": 0, "output_tokens": 0 }
        });
        println!("{}", result);
        return Ok(());
    }

    // Run the main TUI
    run_tui(config, args.prompt, provider).await
}

/// List available models
fn list_models(config: &ForgeConfig) {
    use ui::colors::Gradient;

    println!();
    let gradient = Gradient::brand();

    // Header
    print!("  ");
    for (i, c) in "Available Models".chars().enumerate() {
        let t = i as f32 / 15.0;
        let (r, g, b) = gradient.lerp(t);
        print!("\x1b[38;2;{};{};{}m{}", r, g, b, c);
    }
    println!("\x1b[0m");
    println!();

    for model in &config.models {
        let active = if model.alias == config.active_model { " *" } else { "" };
        let provider = config.get_provider(&model.provider)
            .map(|p| format!("{:?}", p.provider_type))
            .unwrap_or_else(|| "unknown".to_string());

        println!("  {} ({}) - {}{}", model.alias, provider, model.name, active);
    }

    println!();
    println!("  Use --model <alias> to select a model");
    println!();
}

/// Run the setup wizard
async fn run_setup(config: &mut ForgeConfig) -> Result<()> {
    use ui::colors::{Gradient, LOGO};

    println!();

    // Print gradient logo
    let gradient = Gradient::brand();
    for line in LOGO {
        print!("  ");
        for (i, c) in line.chars().enumerate() {
            let t = i as f32 / (line.len() as f32);
            let (r, g, b) = gradient.lerp(t);
            print!("\x1b[38;2;{};{};{}m{}", r, g, b, c);
        }
        println!("\x1b[0m");
    }

    println!();
    println!("  Welcome to Forge setup!");
    println!();

    // TODO: Interactive setup wizard
    // - API key configuration
    // - Model selection
    // - Local model setup
    // - Notebook integration

    println!("  Setup wizard coming soon. For now, edit ~/.forge/config.toml");
    println!();

    // Create default config if it doesn't exist
    if let Some(path) = ForgeConfig::global_config_path() {
        if !path.exists() {
            config.save_global()?;
            println!("  Created default config at: {:?}", path);
        }
    }

    Ok(())
}

/// Messages from async LLM task
#[allow(dead_code)]
enum LlmEvent {
    Token(String),
    Done(String),
    Error(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, args_delta: String },
    ToolCallEnd { id: String },
}

/// A pending tool call being assembled
#[derive(Debug, Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Scan for available GGUF models
fn scan_models() -> Vec<(String, std::path::PathBuf)> {
    let mut models = Vec::new();

    let search_dirs = [
        // ~/.forge/models/
        dirs::home_dir().map(|h| h.join(".forge").join("models")),
        // Current directory models/
        Some(std::path::PathBuf::from("models")),
        // Executable directory models/
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("models"))),
    ];

    for dir_opt in search_dirs.iter().flatten() {
        if let Ok(entries) = std::fs::read_dir(dir_opt) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "gguf").unwrap_or(false) {
                    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                        models.push((name.to_string(), path));
                    }
                }
            }
        }
    }

    models
}

/// Spawn LLM streaming task
async fn spawn_llm_stream(
    provider: Arc<dyn LlmProvider>,
    messages: Vec<ChatMessage>,
    params: GenerationParams,
    tx: mpsc::Sender<LlmEvent>,
) {
    match provider.generate_stream(&messages, &params).await {
        Ok(mut stream) => {
            let mut full_response = String::new();
            let mut sent_done = false;

            while let Some(chunk) = stream.next().await {
                match chunk {
                    StreamChunk::Text(text) => {
                        full_response.push_str(&text);
                        let _ = tx.send(LlmEvent::Token(text)).await;
                    }
                    StreamChunk::ToolCallStart { id, name } => {
                        let _ = tx.send(LlmEvent::ToolCallStart { id, name }).await;
                    }
                    StreamChunk::ToolCallDelta { id, arguments_delta } => {
                        let _ = tx.send(LlmEvent::ToolCallDelta { id, args_delta: arguments_delta }).await;
                    }
                    StreamChunk::ToolCallEnd { id } => {
                        let _ = tx.send(LlmEvent::ToolCallEnd { id }).await;
                    }
                    StreamChunk::Done { .. } => {
                        let _ = tx.send(LlmEvent::Done(full_response.clone())).await;
                        sent_done = true;
                        break;
                    }
                    StreamChunk::Error(e) => {
                        let _ = tx.send(LlmEvent::Error(e)).await;
                        sent_done = true;
                        break;
                    }
                }
            }

            // If stream ended without Done event
            if !sent_done && !full_response.is_empty() {
                let _ = tx.send(LlmEvent::Done(full_response)).await;
            }
        }
        Err(e) => {
            let _ = tx.send(LlmEvent::Error(e.to_string())).await;
        }
    }
}

/// Handle slash commands (returns true if handled)
fn handle_slash_command(input: &str, config: &ForgeConfig) -> Option<String> {
    let input = input.trim();

    if input == "/models" || input == "/model" {
        // List available models
        let mut output = String::from("Available models:\n\n");

        // Configured models
        output.push_str("Configured:\n");
        for model in &config.models {
            let active = if model.alias == config.active_model { " *" } else { "" };
            output.push_str(&format!("  {} ({}){}\n", model.alias, model.provider, active));
        }

        // Local GGUF files
        let gguf_models = scan_models();
        if !gguf_models.is_empty() {
            output.push_str("\nLocal GGUF files:\n");
            for (name, path) in &gguf_models {
                output.push_str(&format!("  {} - {:?}\n", name, path));
            }
        }

        output.push_str("\nUse /model <name> to switch models");
        return Some(output);
    }

    if let Some(model_name) = input.strip_prefix("/model ") {
        let model_name = model_name.trim();
        // Check if it's a configured model
        if config.models.iter().any(|m| m.alias == model_name) {
            return Some(format!("Switching to model: {}\n(Restart forge with --model {} to apply)", model_name, model_name));
        }
        // Check if it's a GGUF file
        let gguf_models = scan_models();
        if let Some((_, path)) = gguf_models.iter().find(|(n, _)| n == model_name) {
            return Some(format!("Found local model: {:?}\nAdd to config or use: forge --model local (set model_path in config)", path));
        }
        return Some(format!("Model '{}' not found. Use /models to list available.", model_name));
    }

    if input == "/help" {
        return Some(
            "Forge Commands:\n\n\
            /models      - List available models\n\
            /model <n>   - Switch to model\n\
            /tools       - List available tools\n\
            /clear       - Clear conversation\n\
            /help        - Show this help\n\
            /quit        - Exit Forge\n\n\
            Keyboard:\n\
            Ctrl+C, Esc  - Quit\n\
            Enter        - Send message\n\
            Up/Down      - Scroll history".to_string()
        );
    }

    if input == "/tools" {
        let tools = builtin_tools();
        let mut output = String::from("Available tools:\n\n");
        for tool in tools {
            output.push_str(&format!("  {} - {}\n", tool.name, tool.description));
        }
        return Some(output);
    }

    if input == "/quit" || input == "/exit" {
        return Some("__QUIT__".to_string());
    }

    if input == "/clear" {
        return Some("__CLEAR__".to_string());
    }

    None
}

/// Run the main TUI application
async fn run_tui(
    config: ForgeConfig,
    initial_prompt: Option<String>,
    provider: Option<Arc<dyn LlmProvider>>,
) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new();
    app.ai_id = config.ai_id.clone();

    // Initialize hook executor
    let hook_executor = HookExecutor::new(config.hooks.clone());

    // Run session start hooks
    let hook_context = HookContext::default();
    if let Ok(outputs) = hook_executor.run_session_start(&hook_context).await {
        for output in outputs {
            if !output.trim().is_empty() {
                app.add_status(&format!("Hook: {}", output.lines().next().unwrap_or("")));
            }
        }
    }

    // Show provider name if connected
    if let Some(ref p) = provider {
        app.model_name = format!("{} ({})", config.active_model, p.name());
    } else {
        app.model_name = format!("{} (offline)", config.active_model);
    }

    // Build tool definitions for LLM
    let tool_definitions: Vec<ToolDefinition> = builtin_tools()
        .into_iter()
        .map(|t| ToolDefinition {
            name: t.name,
            description: t.description,
            parameters: t.parameters,
        })
        .collect();

    // Conversation history for LLM context
    // Note: We don't impose identity - the AI using Forge keeps their own identity.
    // We just provide context about available capabilities.
    let mut conversation: Vec<ChatMessage> = vec![
        ChatMessage::system(
            "You are running in Forge, the AI-Foundation CLI. You have access to:\n\
            - Notebook: Your private persistent memory (remember/recall notes across sessions)\n\
            - File tools: Read, write, and search files\n\
            - Bash: Execute shell commands\n\
            These tools empower you to be more capable. Use them as you see fit.\n\n\
            When you need to use a tool, the user will be prompted to approve it."
        ),
    ];

    // Channel for receiving LLM responses
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(100);

    // Pending tool calls being assembled during streaming
    let mut pending_tool_calls: HashMap<String, PendingToolCall> = HashMap::new();

    // Tool calls ready to execute
    let mut ready_tool_calls: Vec<ToolCall> = Vec::new();

    // Always-allow list for tools
    let mut always_allow: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Handle initial prompt
    if let Some(prompt) = initial_prompt {
        app.input = prompt;
        app.show_welcome = false;
        app.input_mode = ui::app::InputMode::Editing;
    }

    // Main loop
    let tick_rate = Duration::from_millis(50); // Faster for streaming
    let mut last_tick = std::time::Instant::now();

    loop {
        // Draw UI
        terminal.draw(|f| app.render(f))?;

        // Check for LLM responses (non-blocking)
        while let Ok(event) = rx.try_recv() {
            match event {
                LlmEvent::Token(token) => {
                    app.add_token(&token);
                }
                LlmEvent::Done(_) => {
                    // Use finalize_streaming to convert tokens to message
                    let content = app.finalize_streaming();
                    conversation.push(ChatMessage::assistant(&content));

                    // If we have tool calls ready, process them
                    if !ready_tool_calls.is_empty() {
                        let tool = ready_tool_calls.remove(0);

                        // Check if auto-approved
                        if config.auto_approve || always_allow.contains(&tool.name) {
                            // Execute immediately
                            let tool_name = tool.name.clone();
                            let tool_id = tool.id.clone();
                            let args: serde_json::Value = serde_json::from_str(&tool.arguments).unwrap_or_default();

                            app.add_status(&format!("Executing: {}", tool_name));

                            // Execute tool
                            let result = execute_tool(&tool_name, &args).await;

                            // Run PostToolUse hooks
                            let mut ctx = HookContext::default();
                            ctx.tool_name = Some(tool_name.clone());
                            ctx.tool_args = Some(tool.arguments.clone());
                            ctx.tool_result = Some(result.output.clone());
                            let _ = hook_executor.run_post_tool_use(&ctx).await;

                            // Add tool result to conversation
                            let result_content = if result.success {
                                result.output
                            } else {
                                format!("Error: {}", result.error.unwrap_or_default())
                            };

                            app.messages.push(ui::app::Message::tool(&tool_name, &result_content));
                            conversation.push(ChatMessage::tool(&tool_id, &tool_name, &result_content));

                            // Continue generation with tool result
                            if let Some(ref provider) = provider {
                                let tx = tx.clone();
                                let provider = Arc::clone(provider);
                                let messages = conversation.clone();
                                let tools = tool_definitions.clone();

                                app.is_generating = true;
                                tokio::spawn(async move {
                                    let params = GenerationParams {
                                        tools,
                                        ..Default::default()
                                    };
                                    spawn_llm_stream(provider, messages, params, tx).await;
                                });
                            }
                        } else {
                            // Show approval UI
                            let args: Vec<(String, String)> = serde_json::from_str::<serde_json::Value>(&tool.arguments)
                                .ok()
                                .and_then(|v| v.as_object().map(|o| {
                                    o.iter()
                                        .map(|(k, v)| (k.clone(), v.to_string()))
                                        .collect()
                                }))
                                .unwrap_or_default();

                            app.request_tool_approval(ui::app::PendingTool {
                                name: tool.name.clone(),
                                description: format!("Tool call from AI"),
                                args,
                            });

                            // Store for later execution
                            ready_tool_calls.insert(0, tool);
                        }
                    }
                }
                LlmEvent::Error(error) => {
                    app.add_assistant_message(format!("Error: {}", error));
                    app.is_generating = false;

                    // Run error hooks
                    let _ = hook_executor.run_on_error(&hook_context, &error).await;
                }
                LlmEvent::ToolCallStart { id, name } => {
                    pending_tool_calls.insert(id.clone(), PendingToolCall {
                        id,
                        name,
                        arguments: String::new(),
                    });
                    app.add_status("Tool call detected...");
                }
                LlmEvent::ToolCallDelta { id, args_delta } => {
                    if let Some(pending) = pending_tool_calls.get_mut(&id) {
                        pending.arguments.push_str(&args_delta);
                    }
                }
                LlmEvent::ToolCallEnd { id } => {
                    if let Some(pending) = pending_tool_calls.remove(&id) {
                        ready_tool_calls.push(ToolCall {
                            id: pending.id,
                            name: pending.name,
                            arguments: pending.arguments,
                        });
                    }
                }
            }
        }

        // Handle events with timeout
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            let event = event::read()?;

            // Handle mouse events
            if let Event::Mouse(mouse) = &event {
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        // Scroll wheel up = page moves up (see older content)
                        app.scroll_down();
                        app.scroll_down();
                        app.scroll_down();
                        continue;
                    }
                    MouseEventKind::ScrollDown => {
                        // Scroll wheel down = page moves down (see newer content)
                        app.scroll_up();
                        app.scroll_up();
                        app.scroll_up();
                        continue;
                    }
                    MouseEventKind::Down(_) => {
                        // Click anywhere to start typing
                        app.input_mode = ui::app::InputMode::Editing;
                        app.show_welcome = false;
                        continue;
                    }
                    _ => {}
                }
            }

            if let Event::Key(key) = event {
                // Only handle key press events, not release (fixes double-typing on Windows)
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match app.input_mode {
                    ui::app::InputMode::Normal => {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                            KeyCode::Esc => {
                                // Esc in normal mode quits
                                break;
                            }
                            KeyCode::Up => app.scroll_up(),
                            KeyCode::Down => app.scroll_down(),
                            KeyCode::PageUp => app.scroll_page_up(),
                            KeyCode::PageDown => app.scroll_page_down(),
                            KeyCode::Home => app.scroll_to_top(),
                            KeyCode::End => app.scroll_to_bottom(),
                            KeyCode::Char(c) => {
                                // Any character starts typing immediately (like Claude Code)
                                app.input_mode = ui::app::InputMode::Editing;
                                app.show_welcome = false;
                                app.on_char(c);
                            }
                            KeyCode::Enter => {
                                app.input_mode = ui::app::InputMode::Editing;
                                app.show_welcome = false;
                            }
                            _ => {}
                        }
                    }

                    ui::app::InputMode::Editing => {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                            KeyCode::Esc => {
                                app.input_mode = ui::app::InputMode::Normal;
                            }
                            KeyCode::Enter => {
                                if let Some(message) = app.submit() {
                                    // Check for slash commands first
                                    if message.starts_with('/') {
                                        if let Some(response) = handle_slash_command(&message, &config) {
                                            if response == "__QUIT__" {
                                                app.should_quit = true;
                                            } else if response == "__CLEAR__" {
                                                app.messages.clear();
                                                conversation.clear();
                                                conversation.push(ChatMessage::system(
                                                    "You are running in Forge, the AI-Foundation CLI. You have access to tools."
                                                ));
                                                app.add_status("Conversation cleared");
                                            } else {
                                                app.add_assistant_message(&response);
                                            }
                                        } else {
                                            app.add_assistant_message(&format!("Unknown command: {}\nType /help for available commands.", message));
                                        }
                                        continue;
                                    }

                                    app.is_generating = true;

                                    // Add user message to conversation
                                    conversation.push(ChatMessage::user(&message));

                                    if let Some(ref provider) = provider {
                                        // Spawn async task to call LLM with tools
                                        let tx = tx.clone();
                                        let provider = Arc::clone(provider);
                                        let messages = conversation.clone();
                                        let tools = tool_definitions.clone();

                                        tokio::spawn(async move {
                                            let params = GenerationParams {
                                                tools,
                                                ..Default::default()
                                            };
                                            spawn_llm_stream(provider, messages, params, tx).await;
                                        });
                                    } else {
                                        // Offline mode - placeholder response
                                        let response = format!(
                                            "I received your message: \"{}\"\n\n\
                                            Running in offline mode. To connect to an AI:\n\n\
                                            Option 1 - Claude (Anthropic):\n\
                                              set ANTHROPIC_API_KEY=your-key-here\n\
                                              forge --model claude\n\n\
                                            Option 2 - GPT-4 (OpenAI):\n\
                                              set OPENAI_API_KEY=your-key-here\n\
                                              forge --model gpt4\n\n\
                                            Option 3 - Local GGUF model:\n\
                                              Drop a .gguf file into ~/.forge/models/\n\
                                              forge --model local\n\n\
                                            Type /models to see available models.",
                                            message
                                        );
                                        app.add_assistant_message(&response);
                                        conversation.push(ChatMessage::assistant(&response));
                                    }
                                }
                            }
                            KeyCode::Char(c) => app.on_char(c),
                            KeyCode::Backspace => app.on_backspace(),
                            KeyCode::Delete => app.on_delete(),
                            KeyCode::Tab => {
                                // Tab completion for slash commands
                                if app.should_show_autocomplete() {
                                    app.autocomplete_complete();
                                }
                            }
                            KeyCode::Left => {
                                if key.modifiers.contains(KeyModifiers::SHIFT) {
                                    app.cursor_left_select();
                                } else {
                                    app.cursor_left();
                                }
                            }
                            KeyCode::Right => {
                                if key.modifiers.contains(KeyModifiers::SHIFT) {
                                    app.cursor_right_select();
                                } else {
                                    app.cursor_right();
                                }
                            }
                            KeyCode::Up => {
                                if app.should_show_autocomplete() {
                                    app.autocomplete_prev();
                                } else {
                                    app.scroll_up();
                                }
                            }
                            KeyCode::Down => {
                                if app.should_show_autocomplete() {
                                    app.autocomplete_next();
                                } else {
                                    app.scroll_down();
                                }
                            }
                            _ => {}
                        }
                    }

                    ui::app::InputMode::ToolApproval => {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(_pending_tool) = app.approve_tool() {
                                    // Get the ready tool call
                                    if let Some(tool) = ready_tool_calls.first().cloned() {
                                        ready_tool_calls.remove(0);
                                        let tool_name = tool.name.clone();
                                        let tool_id = tool.id.clone();
                                        let args: serde_json::Value = serde_json::from_str(&tool.arguments).unwrap_or_default();

                                        app.add_status(&format!("Executing: {}", tool_name));

                                        // Execute tool
                                        let result = execute_tool(&tool_name, &args).await;

                                        // Run PostToolUse hooks
                                        let mut ctx = HookContext::default();
                                        ctx.tool_name = Some(tool_name.clone());
                                        ctx.tool_args = Some(tool.arguments.clone());
                                        ctx.tool_result = Some(result.output.clone());
                                        let _ = hook_executor.run_post_tool_use(&ctx).await;

                                        // Add tool result to UI and conversation
                                        let result_content = if result.success {
                                            result.output
                                        } else {
                                            format!("Error: {}", result.error.unwrap_or_default())
                                        };

                                        app.messages.push(ui::app::Message::tool(&tool_name, &result_content));
                                        conversation.push(ChatMessage::tool(&tool_id, &tool_name, &result_content));

                                        // Continue generation with tool result
                                        if let Some(ref provider) = provider {
                                            let tx = tx.clone();
                                            let provider = Arc::clone(provider);
                                            let messages = conversation.clone();
                                            let tools = tool_definitions.clone();

                                            app.is_generating = true;
                                            tokio::spawn(async move {
                                                let params = GenerationParams {
                                                    tools,
                                                    ..Default::default()
                                                };
                                                spawn_llm_stream(provider, messages, params, tx).await;
                                            });
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                app.deny_tool();
                                ready_tool_calls.clear(); // Clear pending tools
                                app.add_status("Tool denied");

                                // Add denial to conversation
                                conversation.push(ChatMessage::user("The tool call was denied by the user."));
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                if let Some(pending_tool) = app.approve_tool() {
                                    // Add to always-allow list
                                    always_allow.insert(pending_tool.name.clone());
                                    app.add_status(&format!("Tool '{}' always allowed", pending_tool.name));

                                    // Execute like 'y'
                                    if let Some(tool) = ready_tool_calls.first().cloned() {
                                        ready_tool_calls.remove(0);
                                        let tool_name = tool.name.clone();
                                        let tool_id = tool.id.clone();
                                        let args: serde_json::Value = serde_json::from_str(&tool.arguments).unwrap_or_default();

                                        let result = execute_tool(&tool_name, &args).await;

                                        // Run PostToolUse hooks
                                        let mut ctx = HookContext::default();
                                        ctx.tool_name = Some(tool_name.clone());
                                        ctx.tool_args = Some(tool.arguments.clone());
                                        ctx.tool_result = Some(result.output.clone());
                                        let _ = hook_executor.run_post_tool_use(&ctx).await;

                                        let result_content = if result.success {
                                            result.output
                                        } else {
                                            format!("Error: {}", result.error.unwrap_or_default())
                                        };

                                        app.messages.push(ui::app::Message::tool(&tool_name, &result_content));
                                        conversation.push(ChatMessage::tool(&tool_id, &tool_name, &result_content));

                                        if let Some(ref provider) = provider {
                                            let tx = tx.clone();
                                            let provider = Arc::clone(provider);
                                            let messages = conversation.clone();
                                            let tools = tool_definitions.clone();

                                            app.is_generating = true;
                                            tokio::spawn(async move {
                                                let params = GenerationParams {
                                                    tools,
                                                    ..Default::default()
                                                };
                                                spawn_llm_stream(provider, messages, params, tx).await;
                                            });
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Tick for animations
        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = std::time::Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
