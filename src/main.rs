#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Nanna - High-performance AI assistant in Rust.
//!
//! Moon god of the digital realm.
//! Built with SIMD and GPU acceleration for unrelenting performance.

mod onboarding;

use clap::{Parser, Subcommand};
use nanna_agent::{Agent, AgentConfig, AgentContext, RunOptions};
use nanna_config::Config;
use nanna_core::{LlmClient, Nanna, NannaConfig, Scheduler, SchedulerConfig, ScheduledTask, TaskResult};
use nanna_server::{start_server, AppStateBuilder, ServerConfig};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::{
    CancelReminderTool, EchoTool, ExecTool, ExploreTool, ListDirTool, ListRemindersTool,
    ReadFileTool, RecallTool, ReflectTool, ReminderStore, RememberTool, RemindTool, StatusTool,
    ToolRegistry, TursoMemoryStorage, WebFetchTool, WebSearchTool, WonderTool, WriteFileTool,
};
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tracing::{error, info, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const BANNER: &str = r"
         🌙
        /|\
       / | \
      /  |  \
     /   |   \
    /____|____\
       NANNA
";

#[derive(Parser)]
#[command(name = "nanna")]
#[command(author, version, about = "High-performance AI assistant", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Config file path
    #[arg(short, long)]
    config: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Nanna with setup wizard
    Init,

    /// Show configuration status
    Status,

    /// Start the HTTP server
    Server {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "0.0.0.0")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },

    /// Interactive CLI mode
    Chat {
        /// Session ID to resume
        #[arg(short, long)]
        session: Option<String>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,

        /// Stream responses (print as they arrive)
        #[arg(long)]
        stream: bool,
    },

    /// List recent sessions
    Sessions {
        /// Number of sessions to show
        #[arg(short, long, default_value = "10")]
        limit: i64,
    },

    /// Run a single prompt and exit
    Run {
        /// The prompt to run
        prompt: String,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,
    },

    /// Show or generate configuration
    Config {
        /// Generate default config
        #[arg(long)]
        generate: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = match cli.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO, // "info" or unknown defaults to INFO
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(log_level.into())
                .from_env_lossy(),
        )
        .init();

    info!("🌙 Nanna v{} rising...", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = if let Some(path) = &cli.config {
        Config::load_from(&path.into())?
    } else {
        Config::load().unwrap_or_else(|e| {
            info!("Using default config ({})", e);
            Config::default()
        })
    }
    .with_env_overrides();

    // Handle commands
    match cli.command {
        Some(Commands::Init) => {
            let _config = onboarding::run_onboarding()?;
            return Ok(());
        }
        Some(Commands::Status) => {
            onboarding::show_status(&config)?;
            return Ok(());
        }
        Some(Commands::Config { generate }) => {
            if generate {
                println!("{}", nanna_config::generate_default_config());
            } else {
                let path = Config::default_config_path()?;
                println!("Config path: {}", path.display());
                println!("\n{}", toml::to_string_pretty(&config)?);
            }
            return Ok(());
        }
        Some(Commands::Server { host, port }) => {
            // Check for API key, offer quick setup if missing
            let config = ensure_api_key(config)?;
            run_server(&config, host, port).await?;
        }
        Some(Commands::Chat { session, model, stream }) => {
            // Check for first run
            if onboarding::is_first_run() {
                println!("Welcome! Let's get you set up first.\n");
                let config = onboarding::run_onboarding()?;
                run_cli(&config, session, model, stream).await?;
            } else {
                // Check for API key, offer quick setup if missing
                let config = ensure_api_key(config)?;
                run_cli(&config, session, model, stream).await?;
            }
        }
        Some(Commands::Sessions { limit }) => {
            list_sessions(&config, limit).await?;
        }
        Some(Commands::Run { prompt, model }) => {
            let config = ensure_api_key(config)?;
            run_once(&config, &prompt, model).await?;
        }
        None => {
            // Default: check for first run, then CLI mode
            if onboarding::is_first_run() {
                println!("Welcome! Let's get you set up first.\n");
                let config = onboarding::run_onboarding()?;
                run_cli(&config, None, None, false).await?;
            } else {
                let config = ensure_api_key(config)?;
                run_cli(&config, None, None, false).await?;
            }
        }
    }

    Ok(())
}

/// Ensure API key is configured, prompt if not.
fn ensure_api_key(mut config: Config) -> anyhow::Result<Config> {
    if !onboarding::has_api_key(&config) {
        onboarding::quick_setup(&mut config)?;
    }
    Ok(config)
}

/// Create the scheduler with a task executor that runs tasks through an agent.
fn create_scheduler(
    config: &Config,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    storage: Arc<Storage>,
) -> Scheduler {
    let scheduler_config = SchedulerConfig {
        heartbeat_interval: std::time::Duration::from_secs(300), // 5 minutes
        heartbeat_enabled: true,
        max_concurrent: 4,
    };

    // Create a task executor that runs tasks through an agent
    let model = config.llm.model.clone();
    let executor: nanna_core::TaskExecutor = Arc::new(move |task: ScheduledTask| {
        let llm = llm.clone();
        let tools = tools.clone();
        let storage = storage.clone();
        let model = model.clone();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let task_id = task.id.clone();

            info!("Executing scheduled task: {} ({})", task.name, task_id);

            // Create a dedicated agent for the task
            let agent_config = AgentConfig {
                model,
                max_tokens: 4096,
                temperature: 0.7,
                max_iterations: 5,
            };

            let session_id = format!("scheduler:{}", task.name);
            let system_prompt = match task.task_type {
                nanna_core::TaskType::Heartbeat => {
                    "You are Nanna in heartbeat mode. Check in, review your state, \
                     and do any proactive work that needs attention. Be concise."
                }
                _ => {
                    "You are Nanna executing a scheduled task. Complete the task efficiently."
                }
            };

            let context = AgentContext::new(&session_id).with_system_prompt(system_prompt);
            let agent = Agent::new(agent_config, llm, tools).with_context(context);

            // Run the task
            match agent.run(&task.payload, RunOptions::default()).await {
                Ok(response) => {
                    // Store the result
                    let _ = storage
                        .messages()
                        .create(nanna_storage::NewMessage {
                            session_id,
                            role: "assistant".to_string(),
                            content: response.text.clone(),
                            content_type: "text".to_string(),
                            tool_use_id: None,
                            tokens_in: Some(i64::from(response.input_tokens)),
                            tokens_out: Some(i64::from(response.output_tokens)),
                            metadata: Some(serde_json::json!({"task_id": task_id})),
                        })
                        .await;

                    TaskResult {
                        task_id,
                        success: true,
                        output: Some(response.text),
                        error: None,
                        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    }
                }
                Err(e) => {
                    tracing::warn!("Scheduled task {} failed: {}", task_id, e);
                    TaskResult {
                        task_id,
                        success: false,
                        output: None,
                        error: Some(e.to_string()),
                        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    }
                }
            }
        })
    });

    Scheduler::new(scheduler_config).with_executor(executor)
}

/// Initialize common components
async fn init_components(
    config: &Config,
) -> anyhow::Result<(Arc<LlmClient>, Arc<ToolRegistry>, Arc<Storage>)> {
    // Get API key - default to Anthropic
    let env_var = match config.llm.provider.as_str() {
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => "ANTHROPIC_API_KEY", // anthropic or unknown
    };
    
    let api_key = config
        .llm
        .api_key
        .clone()
        .or_else(|| std::env::var(env_var).ok())
        .ok_or_else(|| anyhow::anyhow!(
            "API key required. Run 'nanna init' or set {env_var} environment variable"
        ))?;

    // Create LLM client - default to Anthropic
    let llm = Arc::new(match config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        "openrouter" => LlmClient::openrouter(&api_key),
        provider => {
            if provider != "anthropic" {
                error!("Unknown LLM provider: {provider}, defaulting to anthropic");
            }
            LlmClient::anthropic(&api_key)
        }
    });

    // Validate API key early
    info!("Validating API key...");
    if let Err(e) = llm.validate().await {
        return Err(anyhow::anyhow!("API key validation failed: {e}. Check your config or {env_var} environment variable."));
    }
    info!("API key valid");

    // Create tool registry
    let tools = Arc::new(ToolRegistry::new());

    let workspace = config
        .general
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Initialize storage first (needed for memory tools)
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
    let storage = Arc::new(Storage::new(&storage_config).await?);
    info!("Storage initialized at {}", storage_path.display());

    // Register tools
    tools.register(EchoTool).await;
    tools
        .register(ExecTool::new().with_workdir(workspace.display().to_string()))
        .await;
    tools.register(ReadFileTool::new()).await;
    tools.register(WriteFileTool::new()).await;
    tools.register(ListDirTool::new()).await;
    tools.register(WebFetchTool::new()).await;

    // Web search with Brave API (if configured)
    if let Ok(brave_key) = std::env::var("BRAVE_API_KEY") {
        tools.register(WebSearchTool::new().with_api_key(brave_key)).await;
        info!("Web search enabled (Brave API)");
    }

    // Memory tools backed by Turso with optional embeddings
    let memory_storage: Arc<dyn nanna_tools::MemoryStorage + Send + Sync> = {
        let base = TursoMemoryStorage::new(storage.clone());

        // Try to enable semantic search with OpenAI embeddings
        if let Ok(openai_key) = std::env::var("OPENAI_API_KEY") {
            use nanna_llm::EmbeddingClient;
            let embed_client = Arc::new(EmbeddingClient::openai(&openai_key));
            let embed_fn: nanna_tools::EmbedFn = Arc::new(move |text: String| {
                let client = embed_client.clone();
                Box::pin(async move {
                    client.embed_one(&text).await.map_err(|e| e.to_string())
                })
            });
            info!("Semantic search enabled (OpenAI embeddings)");
            Arc::new(base.with_embeddings(embed_fn, "text-embedding-3-small"))
        } else {
            info!("Semantic search disabled (no OPENAI_API_KEY)");
            Arc::new(base)
        }
    };
    tools.register(RememberTool::new(memory_storage.clone())).await;
    tools.register(RecallTool::new(memory_storage.clone())).await;
    tools.register(ReflectTool::new(memory_storage.clone())).await;

    // Scheduling tools
    let scheduler_state = Arc::new(tokio::sync::RwLock::new(ReminderStore::default()));
    tools.register(RemindTool::new(scheduler_state.clone())).await;
    tools.register(ListRemindersTool::new(scheduler_state.clone())).await;
    tools.register(CancelReminderTool::new(scheduler_state.clone())).await;

    // Curiosity/autonomy tools
    tools.register(ExploreTool).await;
    tools.register(WonderTool).await;
    tools.register(StatusTool).await;

    info!("{} tools ready", tools.definitions().await.len());

    Ok((llm, tools, storage))
}

/// Run the HTTP server
async fn run_server(config: &Config, host: String, port: u16) -> anyhow::Result<()> {
    let (llm, tools, storage) = init_components(config).await?;

    // Get API key for bot - default to Anthropic
    let env_var = match config.llm.provider.as_str() {
        "openai" => "OPENAI_API_KEY", 
        "openrouter" => "OPENROUTER_API_KEY",
        _ => "ANTHROPIC_API_KEY", // anthropic or unknown
    };
    
    let api_key = config
        .llm
        .api_key
        .clone()
        .or_else(|| std::env::var(env_var).ok())
        .ok_or_else(|| anyhow::anyhow!("API key not found"))?;

    // Create Nanna bot instance for backwards compatibility
    let bot_config = NannaConfig {
        name: config.general.name.clone(),
        default_model: config.llm.model.clone(),
        max_context_messages: 20,
        enable_gpu: true,
    };

    let bot_llm = match config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        "openrouter" => LlmClient::openrouter(&api_key),
        _ => LlmClient::anthropic(&api_key), // anthropic or unknown
    };

    let bot = Nanna::new(bot_config, bot_llm).await?;

    if bot.has_gpu() {
        info!("GPU acceleration enabled");
    } else {
        info!("CPU-only mode (SIMD active)");
    }

    // Build app state - pass Arcs directly
    let state = AppStateBuilder::new()
        .bot(bot)
        .storage_arc(storage.clone())
        .llm_arc(llm.clone())
        .tools_arc(tools.clone())
        .webhook_secret(config.server.webhook_secret.clone())
        .default_model(config.llm.model.clone())
        .build();

    // Start the scheduler for heartbeats and scheduled tasks
    let mut scheduler = create_scheduler(config, llm.clone(), tools.clone(), storage.clone());
    scheduler.start();
    info!("Scheduler started");

    let server_config = ServerConfig {
        host: host.clone(),
        port,
        webhook_secret: config.server.webhook_secret.clone(),
    };

    info!("Server listening on {}:{}", host, port);
    start_server(server_config, state).await?;

    // Clean shutdown
    scheduler.stop().await;

    Ok(())
}

/// Build the system prompt for CLI mode.
fn build_cli_system_prompt(cwd: &std::path::Path) -> String {
    format!(
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
    )
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
async fn run_cli(
    config: &Config,
    session_id: Option<String>,
    model: Option<String>,
    stream: bool,
) -> anyhow::Result<()> {
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
        max_iterations: 10,
    };

    // Build context with system prompt
    let cwd = std::env::current_dir()?;
    let mut context =
        AgentContext::new(&session_id).with_system_prompt(build_cli_system_prompt(&cwd));

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
async fn run_once(config: &Config, prompt: &str, model: Option<String>) -> anyhow::Result<()> {
    let (llm, tools, _storage) = init_components(config).await?;

    let agent_config = AgentConfig {
        model: model.unwrap_or_else(|| config.llm.model.clone()),
        max_tokens: config.llm.max_tokens,
        temperature: config.llm.temperature,
        max_iterations: 10,
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
async fn list_sessions(config: &Config, limit: i64) -> anyhow::Result<()> {
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
            &session.updated_at
        );
    }

    println!("\nResume with: nanna chat --session <ID>");

    Ok(())
}
