import sys
sys.stdout.reconfigure(encoding='utf-8')

f = 'gui/src-tauri/src/daemon_client.rs'
lines = open(f, encoding='utf-8').readlines()

for i, line in enumerate(lines):
    # Change chat_send signature to accept attachments
    if 'pub async fn chat_send(&self, session_id: &str, content: &str)' in line:
        lines[i] = '    pub async fn chat_send(&self, session_id: &str, content: &str, attachments: Vec<serde_json::Value>) -> Result<Value, String> {\n'
        print(f"Fixed chat_send signature at line {i+1}")
    # Change attachments: [] to attachments: attachments
    if '"attachments": []' in line and 'chat' in lines[i-5:i+1].__repr__():
        lines[i] = line.replace('"attachments": []', '"attachments": attachments')
        print(f"Fixed attachments value at line {i+1}")

open(f, 'w', encoding='utf-8').writelines(lines)
print("daemon_client.rs updated")
