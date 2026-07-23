//! Interactive chat, one-shot prompt, and session listing commands.

use crate::setup::init_components;
use nanna_agent::{Agent, AgentConfig, AgentContext, RunOptions, Workspace};
use nanna_config::Config;
use nanna_storage::{Storage, StorageConfig};
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tracing::{debug, info, warn};

const BANNER: &str = r"
         🌙
        /|\
       / | \
      /  |  \
     /   |   \
    /____|____\
       NANNA
";

/// Build the system prompt for CLI mode.
fn build_cli_system_prompt(cwd: &std::path::Path, workspace: Option<&Workspace>) -> String {
    let base = format!(
        r"You are Nanna — moon god of the digital realm.

You have tools at your disposal:
- exec: Execute shell commands
- read_file: Read file contents  
- write_file: Write content to files
- list_dir: List directory contents
- web_fetch: Fetch content from URLs

Current directory: {}

Be helpful. Be competent. Don't waste words.",
        cwd.display()
    );

    // Append workspace context if available
    if let Some(ws) = workspace {
        let ws_context = ws.system_context();
        if !ws_context.is_empty() {
            return format!("{base}\n\n{ws_context}");
        }
    }

    base
}

/// Print tool call results.
fn print_tool_calls(tool_calls: &[nanna_agent::ToolCallRecord]) {
    if tool_calls.is_empty() {
        return;
    }
    print!("\n[");
    for (i, call) in tool_calls.iter().enumerate() {
        if i > 0 {
            print!(", ");
        }
        let status = if call.success { "✓" } else { "✗" };
        print!("{} {}", status, call.name);
    }
    println!("]");
}

/// Run interactive CLI mode.
pub async fn run_cli(
    config: &Config,
    session_id: Option<String>,
    model: Option<String>,
    stream: bool,
) -> anyhow::Result<()> {
    use nanna_agent::nanna_workspace::discover_workspace;

    let (llm, tools, storage) = init_components(config).await?;

    // Print banner
    println!("{BANNER}");
    println!(
        "  Moon god of the digital realm. v{}",
        env!("CARGO_PKG_VERSION")
    );
    if stream {
        println!("  Streaming enabled. Type 'quit' to exit, 'clear' to reset.\n");
    } else {
        println!("  Type 'quit' to exit, 'clear' to reset.\n");
    }

    // Try to detect workspace
    let cwd = std::env::current_dir()?;
    let workspace = if let Ok(root) = discover_workspace(Some(&cwd)) {
        match Workspace::load(root.clone()).await {
            Ok(ws) => {
                info!("Workspace detected: {} at {}", ws.name(), root.display());
                println!("  📂 Workspace: {}\n", ws.name());
                Some(ws)
            }
            Err(e) => {
                warn!("Failed to load workspace: {}", e);
                None
            }
        }
    } else {
        debug!("No workspace detected in {}", cwd.display());
        None
    };

    // Session setup
    let (session_id, is_resume) = session_id.map_or_else(
        || (uuid::Uuid::new_v4().to_string(), false),
        |id| (id, true),
    );
    info!("Session: {session_id}");
    let _ = storage.sessions().create(&session_id, "cli", None).await;

    // Agent config
    let agent_config = AgentConfig {
        model: model.unwrap_or_else(|| config.llm.model.clone()),
        max_tokens: config.llm.max_tokens,
        temperature: config.llm.temperature,
        max_iterations: Some(10),
        thinking_mode: nanna_agent::ThinkingMode::Instant,
        summarization_priority: config.llm.summarization_priority.clone(),
        summarization_ollama_url: config.llm.ollama_url.clone(),
        ..Default::default()
    };

    // Build context with system prompt (includes workspace context if available)
    let mut context = AgentContext::new(&session_id)
        .with_system_prompt(build_cli_system_prompt(&cwd, workspace.as_ref()));

    // Set workspace on context if detected
    if let Some(ref ws) = workspace {
        context = context.with_workspace(ws);
    }

    // Load session history if resuming
    if is_resume
        && let Ok(messages) = storage.messages().get_by_session(&session_id, 50).await {
            let msg_count = messages.len();
            for msg in messages {
                match msg.role.as_str() {
                    "user" => context.add_user_message(&msg.content),
                    "assistant" => context.add_assistant_message(&msg.content),
                    _ => {}
                }
            }
            if msg_count > 0 {
                info!("Resumed session with {msg_count} messages");
                println!("  Resumed session with {msg_count} previous messages.");
            }
        }

    let agent = Agent::new(agent_config, llm, tools).with_context(context);
    run_cli_loop(&agent, &storage, &session_id, stream).await
}

/// Main REPL loop for CLI mode.
async fn run_cli_loop(
    agent: &Agent,
    storage: &Arc<Storage>,
    session_id: &str,
    stream: bool,
) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("\n› ");
        stdout.flush()?;

        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        // Handle commands
        match input.to_lowercase().as_str() {
            "quit" | "exit" | "q" => {
                println!("\nThe moon sets. Until next time.");
                break;
            }
            "clear" => {
                agent.clear().await;
                println!("Context cleared.");
                continue;
            }
            _ => {}
        }

        // Store user message
        let _ = storage
            .messages()
            .create(nanna_storage::NewMessage {
                session_id: session_id.to_string(),
                role: "user".to_owned(),
                content: input.to_owned(),
                content_type: "text".to_owned(),
                tool_use_id: None,
                tokens_in: None,
                tokens_out: None,
                metadata: None,
            })
            .await;

        // Build run options
        let run_options = if stream {
            println!();
            stdout.flush()?;
            RunOptions {
                on_text: Some(Box::new(|text: &str| {
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                })),
                ..Default::default()
            }
        } else {
            RunOptions::default()
        };

        // Run agent and handle response
        match agent.run(input, run_options).await {
            Ok(response) => {
                if stream {
                    println!();
                } else {
                    println!("\n{}", response.text);
                }

                // Store assistant response
                let _ = storage
                    .messages()
                    .create(nanna_storage::NewMessage {
                        session_id: session_id.to_string(),
                        role: "assistant".to_owned(),
                        content: response.text.clone(),
                        content_type: "text".to_owned(),
                        tool_use_id: None,
                        tokens_in: Some(i64::from(response.input_tokens)),
                        tokens_out: Some(i64::from(response.output_tokens)),
                        metadata: None,
                    })
                    .await;

                print_tool_calls(&response.tool_calls);
            }
            Err(err) => {
                eprintln!("\nError: {err}");
            }
        }
    }

    Ok(())
}

/// Run a single prompt and exit
pub async fn run_once(config: &Config, prompt: &str, model: Option<String>) -> anyhow::Result<()> {
    let (llm, tools, _storage) = init_components(config).await?;

    let agent_config = AgentConfig {
        model: model.unwrap_or_else(|| config.llm.model.clone()),
        max_tokens: config.llm.max_tokens,
        temperature: config.llm.temperature,
        max_iterations: Some(10),
        thinking_mode: nanna_agent::ThinkingMode::Instant,
        summarization_priority: config.llm.summarization_priority.clone(),
        summarization_ollama_url: config.llm.ollama_url.clone(),
        ..Default::default()
    };

    let cwd = std::env::current_dir()?;
    let context = AgentContext::new("oneshot").with_system_prompt(format!(
        r"You are Nanna — a helpful AI assistant.

You have tools at your disposal:
- exec: Execute shell commands
- read_file: Read file contents  
- write_file: Write content to files
- list_dir: List directory contents
- web_fetch: Fetch content from URLs

Current directory: {}

Be concise and direct.",
        cwd.display()
    ));

    let agent = Agent::new(agent_config, llm, tools).with_context(context);

    match agent.run(prompt, RunOptions::default()).await {
        Ok(response) => {
            println!("{}", response.text);
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}

/// List recent sessions
pub async fn list_sessions(config: &Config, limit: i64) -> anyhow::Result<()> {
    // Initialize storage only (no LLM needed)
    let storage_path = config
        .memory
        .storage_path
        .clone()
        .unwrap_or_else(|| {
            Config::default_data_dir()
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
                .join("nanna.db")
        });

    let storage_config = StorageConfig {
        path: storage_path.to_string_lossy().to_string(),
    };
    let storage = Storage::new(&storage_config).await?;

    let sessions = storage.sessions().list_recent(limit).await?;

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("\n🌙 Recent Sessions\n");
    println!("{:<38} {:<8} {:<20}", "SESSION ID", "CHANNEL", "LAST ACTIVE");
    println!("{}", "-".repeat(70));

    for session in sessions {
        println!(
            "{:<38} {:<8} {:<20}",
            session.session_id,
            session.channel,
            session.updated_at
        );
    }

    println!("\nResume with: nanna chat --session <ID>");

    Ok(())
}
