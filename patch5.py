import sys
sys.stdout.reconfigure(encoding='utf-8')

f = 'gui/src-tauri/src/backend.rs'
lines = open(f, encoding='utf-8').readlines()

for i, line in enumerate(lines):
    if 'pub async fn chat_send(&self, session_id: &str, content: &str)' in line:
        lines[i] = line.replace(
            'pub async fn chat_send(&self, session_id: &str, content: &str)',
            'pub async fn chat_send(&self, session_id: &str, content: &str, attachments: Vec<serde_json::Value>)'
        )
        print(f'Fixed chat_send signature at line {i+1}')
    if 'self.daemon_client.chat_send(session_id, content).await' in line:
        lines[i] = line.replace(
            'self.daemon_client.chat_send(session_id, content).await',
            'self.daemon_client.chat_send(session_id, content, attachments).await'
        )
        print(f'Fixed daemon_client call at line {i+1}')

open(f, 'w', encoding='utf-8').writelines(lines)
print('backend.rs updated')
