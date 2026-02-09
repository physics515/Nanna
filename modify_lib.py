#!/usr/bin/env python3

import re

# Read the file with UTF-8 encoding
filepath = r"D:\Development\nanna\gui\src-tauri\src\lib.rs"

with open(filepath, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# Find the line containing the target string
target_line = "    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,"
insert_index = -1

for i, line in enumerate(lines):
    if target_line in line:
        insert_index = i
        break

if insert_index == -1:
    print("ERROR: Could not find target line")
    exit(1)

# Insert two new lines after the target line
new_lines = [
    "    /// Current streaming cancellation token\n",
    "    stream_cancel_token: Arc<tokio::sync::Mutex<Option<tokio::sync::CancellationToken>>>,\n"
]

lines.insert(insert_index + 1, new_lines[0])
lines.insert(insert_index + 2, new_lines[1])

# Write the file back
with open(filepath, 'w', encoding='utf-8') as f:
    f.writelines(lines)

print(f"✓ File modified: inserted 2 lines after line {insert_index + 1}")

# Find the line number of #[tauri::command] before async fn send_message
send_message_index = -1
tauri_command_index = -1

for i, line in enumerate(lines):
    if "async fn send_message" in line:
        send_message_index = i
        break

if send_message_index == -1:
    print("ERROR: Could not find 'async fn send_message'")
    exit(1)

# Search backwards from send_message to find #[tauri::command]
for i in range(send_message_index - 1, -1, -1):
    if "#[tauri::command]" in lines[i]:
        tauri_command_index = i
        break

if tauri_command_index == -1:
    print("ERROR: Could not find #[tauri::command] before send_message")
    exit(1)

print(f"✓ #[tauri::command] line number: {tauri_command_index + 1}")
