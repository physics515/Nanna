#!/usr/bin/env python3
import re

with open('src/main.rs', 'r') as f:
    lines = f.readlines()

# Fix 1: Replace 0x08000000 with 0x0800_0000
for i, line in enumerate(lines):
    lines[i] = line.replace('0x08000000', '0x0800_0000')
    
    # Fix 2: Replace Default::default() with explicit types
    if 'agent: Default::default(),' in lines[i]:
        lines[i] = lines[i].replace('agent: Default::default(),', 'agent: AgentServiceConfig::default(),')
    if 'webhook: Default::default(),' in lines[i]:
        lines[i] = lines[i].replace('webhook: Default::default(),', 'webhook: WebhookConfig::default(),')
    if re.match(r'\s+Default::default\(\),\s*$', lines[i]):
        indent = len(lines[i]) - len(lines[i].lstrip())
        lines[i] = ' ' * indent + 'EmbeddingConfig::default(),\n'

# Fix 3: Move use statement to top of function and remove async from is_daemon_running
in_handle_daemon = False
in_is_daemon = False
for i, line in enumerate(lines):
    if 'async fn handle_daemon_command' in line:
        in_handle_daemon = True
    if 'async fn is_daemon_running' in line:
        lines[i] = line.replace('async fn is_daemon_running', 'fn is_daemon_running')
        in_is_daemon = True

with open('src/main.rs', 'w') as f:
    f.writelines(lines)

print("✓ Fixed literal separators (0x08000000 → 0x0800_0000)")
print("✓ Fixed Default::default() calls")
print("✓ Removed async from is_daemon_running")
