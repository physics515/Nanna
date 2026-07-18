//! `workspace` subcommand handlers.

use crate::WorkspaceAction;
use nanna_agent::Workspace;

/// Print workspace status details
async fn print_workspace_status() -> anyhow::Result<()> {
    use nanna_agent::nanna_workspace::discover_workspace;

    let cwd = std::env::current_dir()?;

    if let Ok(root) = discover_workspace(Some(&cwd)) {
        let workspace = Workspace::load(root.clone()).await?;

        println!("Workspace Status\n");
        println!("   Root: {}", root.display());
        println!("   Name: {}", workspace.name());
        println!("   Marker: {:?}", workspace.marker);
        println!("\nContext Files:");

        let files = &workspace.files;
        if files.readme.as_ref().is_some_and(|f| f.exists) {
            println!("   + README.md");
        }
        if files.agents.as_ref().is_some_and(|f| f.exists) {
            println!("   + AGENTS.md");
        }
        if files.contributing.as_ref().is_some_and(|f| f.exists) {
            println!("   + CONTRIBUTING.md");
        }
        if files.roadmap.as_ref().is_some_and(|f| f.exists) {
            println!("   + ROADMAP.md");
        }

        println!("\nContext Size:");
        println!(
            "   {} bytes (~{} tokens)",
            files.total_size(),
            files.estimated_tokens()
        );
    } else {
        println!("No workspace found in current directory.");
        println!("   Run 'nanna workspace init' to create one.");
    }

    Ok(())
}

/// Handle workspace subcommands
pub(crate) async fn handle_workspace_command(action: WorkspaceAction) -> anyhow::Result<()> {
    use nanna_agent::nanna_workspace::{create_from_template, discover_workspace, list_templates};

    match action {
        WorkspaceAction::Init { template, path } => {
            let target =
                path.map_or_else(|| std::env::current_dir().unwrap_or_default(), std::path::PathBuf::from);

            println!("Initializing workspace at {}", target.display());

            // Only treat as existing if standard context is already present
            if target.join("AGENTS.md").exists() {
                println!("AGENTS.md already exists at {}", target.display());
                println!("   Use 'nanna workspace status' to see details.");
                return Ok(());
            }

            create_from_template(&target, &template).await?;

            println!("Created workspace with '{template}' template");
            println!("\nFiles created:");
            for e in std::fs::read_dir(&target)?.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".md") || name == ".nanna" {
                    println!("   - {name}");
                }
            }
            println!("\nRun 'nanna chat' to start chatting in this workspace!");
            println!("Persona/user profile: global config agent.persona / agent.user_profile");
            println!("Memory: DB store (no MEMORY.md sidecar)");
        }

        WorkspaceAction::Status => {
            print_workspace_status().await?;
        }

        WorkspaceAction::Templates => {
            let templates = list_templates();

            println!("Available Workspace Templates\n");
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
                    println!("Reloaded workspace: {}", workspace.name());
                    println!("   {} files loaded", workspace.files.existing_files().len());
                }
                Err(_) => {
                    println!("No workspace found in current directory.");
                }
            }
        }
    }

    Ok(())
}
