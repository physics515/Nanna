//! `workspace` subcommand handlers.

use crate::WorkspaceAction;
use nanna_agent::Workspace;

/// Print workspace status details
async fn print_workspace_status() -> anyhow::Result<()> {
    use nanna_agent::nanna_workspace::discover_workspace;

    let cwd = std::env::current_dir()?;

    if let Ok(root) = discover_workspace(Some(&cwd)) {
        let workspace = Workspace::load(root.clone()).await?;

        println!("🌙 Workspace Status\n");
        println!("   Root: {}", root.display());
        println!("   Name: {}", workspace.name());
        println!("   Marker: {:?}", workspace.marker);
        println!("\n📁 Context Files:");

        let files = &workspace.files;
        if files.agents.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ AGENTS.md");
        }
        if files.soul.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ SOUL.md");
        }
        if files.user.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ USER.md");
        }
        if files.tools.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ TOOLS.md");
        }
        if files.memory.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ MEMORY.md");
        }
        if files.identity.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ IDENTITY.md");
        }
        if files.heartbeat.as_ref().is_some_and(|f| f.exists) {
            println!("   ✓ HEARTBEAT.md");
        }
        if files.bootstrap.as_ref().is_some_and(|f| f.exists) {
            println!("   ⚡ BOOTSTRAP.md (fresh workspace)");
        }

        if !files.daily_memories.is_empty() {
            println!("\n📅 Recent Daily Notes:");
            for daily in &files.daily_memories {
                println!("   - {}", daily.name);
            }
        }

        println!("\n📊 Context Size:");
        println!("   {} bytes (~{} tokens)",
            files.total_size(),
            files.estimated_tokens()
        );
    } else {
        println!("❌ No workspace found in current directory.");
        println!("   Run 'nanna workspace init' to create one.");
    }

    Ok(())
}

/// Handle workspace subcommands
pub(crate) async fn handle_workspace_command(action: WorkspaceAction) -> anyhow::Result<()> {
    use nanna_agent::nanna_workspace::{
        create_from_template, discover_workspace, list_templates,
    };

    match action {
        WorkspaceAction::Init { template, path } => {
            let target = path.map_or_else(|| std::env::current_dir().unwrap_or_default(), std::path::PathBuf::from);

            println!("🌙 Initializing workspace at {}", target.display());

            // Check if workspace already exists
            if discover_workspace(Some(&target)).is_ok() {
                println!("⚠️  Workspace already exists at {}", target.display());
                println!("   Use 'nanna workspace status' to see details.");
                return Ok(());
            }

            // Create from template
            create_from_template(&target, &template).await?;

            println!("✅ Created workspace with '{template}' template");
            println!("\n📁 Files created:");
            for e in std::fs::read_dir(&target)?.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".md") || name.starts_with('.') {
                    println!("   - {name}");
                }
            }
            println!("\n🚀 Run 'nanna chat' to start chatting in this workspace!");
        }

        WorkspaceAction::Status => {
            print_workspace_status().await?;
        }

        WorkspaceAction::Templates => {
            let templates = list_templates();
            
            println!("🌙 Available Workspace Templates\n");
            for t in templates {
                println!("   {} - {}", t.id, t.name);
                println!("      {}", t.description);
                println!();
            }
            println!("Use: nanna workspace init --template <id>");
        }

        WorkspaceAction::Reload => {
            let cwd = std::env::current_dir()?;
            
            match discover_workspace(Some(&cwd)) {
                Ok(root) => {
                    let workspace = Workspace::load(root).await?;
                    println!("✅ Reloaded workspace: {}", workspace.name());
                    println!("   {} files loaded", workspace.files.existing_files().len());
                }
                Err(_) => {
                    println!("❌ No workspace found in current directory.");
                }
            }
        }
    }

    Ok(())
}
