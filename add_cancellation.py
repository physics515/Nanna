#!/usr/bin/env python3
import re

file_path = r"D:\Development\nanna\gui\src-tauri\src\lib.rs"

# Read the file
with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Add stream_cancel_token field to AppState
embedded_run_states_pattern = r"(\s+embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,)\n"
replacement = r"\1\n    /// Current streaming cancellation token\n    stream_cancel_token: Arc<tokio::sync::Mutex<Option<tokio::sync::CancellationToken>>>,\n"
content = re.sub(embedded_run_states_pattern, replacement, content)

# 2. Add cancel_message command before the last command or before the setup function
# Find a good place - after send_message function
cancel_command = '''/// Cancel the current streaming message
#[tauri::command]
async fn cancel_message(state: State<'_, Arc<RwLock<AppState>>>) -> Result<(), String> {
    let state_guard = state.read().await;
    if let Some(token) = state_guard.stream_cancel_token.lock().await.as_ref() {
        token.cancel();
    }
    Ok(())
}

'''

# Find the location to insert (before the last Tauri command or before setup)
# Look for the pattern of a command definition
setup_pattern = r"(fn main\(\) -> tauri::Result<\(\)>)"
match = re.search(setup_pattern, content)
if match:
    insert_pos = match.start()
    content = content[:insert_pos] + cancel_command + content[insert_pos:]
    print("✓ Added cancel_message command")
else:
    print("✗ Could not find insertion point for cancel_message command")

# Write the file back
with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("✓ Modified lib.rs successfully")
