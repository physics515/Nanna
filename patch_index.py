import sys
sys.stdout.reconfigure(encoding='utf-8')

f = 'gui/app/pages/index.vue'
lines = open(f, encoding='utf-8').readlines()
changes = 0

for i, line in enumerate(lines):
    # Add chatInputRef declaration after existing refs
    if 'const input = ref' in line and 'chatInputRef' not in lines[i-1]:
        # Check if chatInputRef already exists nearby
        nearby = ''.join(lines[max(0,i-5):i+5])
        if 'chatInputRef' not in nearby:
            lines.insert(i+1, "const chatInputRef = ref<any>(null)\n")
            changes += 1
            break

# Re-read after potential insert
for i, line in enumerate(lines):
    # Add ref to ChatInput component
    if '<ChatInput' in line and 'ref=' not in line:
        lines[i] = line.replace('<ChatInput', '<ChatInput\n          ref="chatInputRef"')
        changes += 1
        break

for i, line in enumerate(lines):
    # Update sendMessage to get attachments
    if "const userMessage = input.value.trim()" in line:
        # Check if already patched
        if i+1 < len(lines) and 'getAttachments' not in lines[i+1]:
            lines.insert(i+1, "  const imageAttachments = chatInputRef.value?.getAttachments?.() || []\n")
            changes += 1
            break

for i, line in enumerate(lines):
    # Update sendMessageToBackend call in sendMessage
    if 'await sendMessageToBackend(userMessage)' in line and 'imageAttachments' not in line:
        lines[i] = line.replace('sendMessageToBackend(userMessage)', 'sendMessageToBackend(userMessage, imageAttachments)')
        changes += 1
        break

for i, line in enumerate(lines):
    # Update sendMessageToBackend signature
    if 'async function sendMessageToBackend(message: string)' in line and 'attachments' not in line:
        lines[i] = line.replace(
            'async function sendMessageToBackend(message: string)',
            'async function sendMessageToBackend(message: string, attachments: Array<{filename: string, content_type: string, data: string}> = [])'
        )
        changes += 1
        break

for i, line in enumerate(lines):
    # Update invoke call to include attachments
    if "await invoke('send_message'," in line and 'attachments' not in lines[i+1] and 'attachments' not in line:
        # Find the closing of the invoke params
        for j in range(i, min(i+5, len(lines))):
            if 'message' in lines[j] and 'attachments' not in lines[j]:
                lines[j] = lines[j].rstrip().rstrip(')').rstrip('\n')
                if lines[j].rstrip().endswith('}'):
                    # message is on same line as closing brace
                    pass
                break
        # Find the line with just 'message' or 'message\n    })'
        for j in range(i, min(i+5, len(lines))):
            if 'message' in lines[j] and 'sessionId' not in lines[j]:
                if 'attachments' not in lines[j]:
                    lines[j] = lines[j].replace('message', 'message,\n      attachments')
                    changes += 1
                break
        break

for i, line in enumerate(lines):
    # Update processNextQueuedMessage
    if 'await sendMessageToBackend(next.content)' in line and 'attachments' not in line:
        lines[i] = line.replace('sendMessageToBackend(next.content)', 'sendMessageToBackend(next.content, [])')
        changes += 1
        break

open(f, 'w', encoding='utf-8').writelines(lines)
print(f"index.vue updated ({changes} changes)")
