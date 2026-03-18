import sys
sys.stdout.reconfigure(encoding='utf-8')
f = 'gui/src-tauri/src/lib.rs'
lines = open(f, encoding='utf-8').readlines()
changes = 0

for i, line in enumerate(lines):
    # Fix send_message_daemon signature (add attachments param)
    if 'async fn send_message_daemon(' in line:
        # Find the closing paren of params - look for 'message: String,'
        for j in range(i, min(i+10, len(lines))):
            if '    message: String,' in lines[j]:
                lines[j] = lines[j].rstrip('\n') + '\n    attachments: Vec<serde_json::Value>,\n'
                # But wait, there's already a line after. We need to insert, not append.
                # Actually let's just add the param after message
                lines[j] = '    message: String,\n    attachments: Vec<serde_json::Value>,\n'
                changes += 1
                print(f"Added attachments param to send_message_daemon at line {j+1}")
                break
        break

# Fix the chat_send call inside send_message_daemon
for i, line in enumerate(lines):
    if 'state.backend.chat_send(&session_id, &message).await' in line:
        lines[i] = line.replace(
            'state.backend.chat_send(&session_id, &message).await',
            'state.backend.chat_send(&session_id, &message, attachments).await'
        )
        changes += 1
        print(f"Fixed chat_send call at line {i+1}")
        break

# Fix send_message command signature - add attachments param
for i, line in enumerate(lines):
    if '#[tauri::command]' in line:
        # Check if next function is send_message
        for j in range(i+1, min(i+5, len(lines))):
            if 'async fn send_message(' in lines[j]:
                # Find the message: String line
                for k in range(j, min(j+10, len(lines))):
                    if '    message: String,' in lines[k]:
                        lines[k] = '    message: String,\n    attachments: Option<Vec<serde_json::Value>>,\n'
                        changes += 1
                        print(f"Added attachments param to send_message at line {k+1}")
                        break
                break

# Fix the call to send_message_daemon inside send_message
for i, line in enumerate(lines):
    if 'send_message_daemon(&app, &state_guard, session_id, message).await' in line:
        lines[i] = line.replace(
            'send_message_daemon(&app, &state_guard, session_id, message).await',
            'send_message_daemon(&app, &state_guard, session_id, message, attachments.unwrap_or_default()).await'
        )
        changes += 1
        print(f"Fixed send_message_daemon call at line {i+1}")
        break

# Also need to handle the non-daemon path - find where send_message routes
# to embedded mode. Search for the else branch after daemon check.
# The embedded path likely doesn't use attachments yet, but we need to
# make sure the variable is consumed or the compiler will warn.

open(f, 'w', encoding='utf-8').writelines(lines)
print(f"lib.rs updated ({changes} changes)")
