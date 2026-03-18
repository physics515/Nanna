import sys
sys.stdout.reconfigure(encoding='utf-8')
f = 'crates/nanna-daemon/src/control.rs'
lines = open(f, encoding='utf-8').readlines()

# 1. Change "attachments: _" to "attachments" on the ChatAction::Send line
for i, line in enumerate(lines):
    if 'ChatAction::Send { session_id, content, attachments: _ }' in line:
        lines[i] = line.replace('attachments: _', 'attachments')
        print(f'Fixed line {i+1}: attachments binding')
        break

# 2. Find the chat_in_workspace call and add image_attachments conversion before it
for i, line in enumerate(lines):
    if 'agent.chat_in_workspace(&session_id, &content, Some(system_prompt), &prior_messages, effective_ws_id.clone())' in line:
        indent = '                '
        insertion = (
            indent + '// Convert protocol attachments to (base64_data, media_type) tuples\n' +
            indent + 'let image_attachments: Vec<(String, String)> = attachments.into_iter()\n' +
            indent + '    .filter(|a| a.content_type.starts_with("image/"))\n' +
            indent + '    .map(|a| (a.data, a.content_type))\n' +
            indent + '    .collect();\n'
        )
        # Replace the line to add image_attachments argument
        lines[i] = line.replace(
            'agent.chat_in_workspace(&session_id, &content, Some(system_prompt), &prior_messages, effective_ws_id.clone())',
            'agent.chat_in_workspace(&session_id, &content, Some(system_prompt), &prior_messages, effective_ws_id.clone(), image_attachments)'
        )
        lines.insert(i, insertion)
        print(f'Fixed line {i+1}: added image_attachments conversion')
        break

open(f, 'w', encoding='utf-8').writelines(lines)
print('control.rs updated')
