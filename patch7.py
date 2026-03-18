import sys
sys.stdout.reconfigure(encoding='utf-8')

# Fix 1: agent_service.rs line 610 - clone attachments
f = 'crates/nanna-daemon/src/agent_service.rs'
lines = open(f, encoding='utf-8').readlines()
for i, line in enumerate(lines):
    if line.strip() == 'attachments,' and i > 600:
        lines[i] = line.replace('attachments,', 'attachments: attachments.clone(),')
        print(f'Fixed clone at line {i+1}')
        break
open(f, 'w', encoding='utf-8').writelines(lines)

# Fix 2: control.rs lines 825 and 831 - add vec![] argument
f = 'crates/nanna-daemon/src/control.rs'
lines = open(f, encoding='utf-8').readlines()
changes = 0
for i, line in enumerate(lines):
    if 'chat_with_options(' in line and 'max_iters, None)' in line:
        lines[i] = line.replace('max_iters, None)', 'max_iters, None, vec![])')
        changes += 1
        print(f'Fixed chat_with_options call at line {i+1}')
    elif 'chat_with_options(' in line and 'max_iters, None).await' in line:
        lines[i] = line.replace('max_iters, None).await', 'max_iters, None, vec![]).await')
        changes += 1
        print(f'Fixed chat_with_options call at line {i+1}')
open(f, 'w', encoding='utf-8').writelines(lines)
print(f'control.rs: {changes} fixes applied')
