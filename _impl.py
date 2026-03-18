# Phase 7 remaining features implementation script
import sys, os, codecs, json

sys.stdout = codecs.getwriter('utf-8')(sys.stdout.buffer)
os.chdir(r'D:\Development\nanna')

def read_file(path):
    with open(path, 'r', encoding='utf-8') as f:
        return f.read()

def write_file(path, content):
    os.makedirs(os.path.dirname(path) or '.', exist_ok=True)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(content)
    print(f'  Wrote {path} ({len(content)} bytes)')

# Step 1: Read current files
print('=== Reading current files ===')
chatinput = read_file('gui/app/components/ChatInput.vue')
print(f'  ChatInput.vue: {len(chatinput.splitlines())} lines')

# Show the import section and editor config
lines = chatinput.splitlines()
for i, line in enumerate(lines):
    if 'import' in line.lower() or 'extensions' in line.lower() or 'useEditor' in line or 'StarterKit' in line or 'Table' in line or 'TaskList' in line:
        print(f'  {i+1}: {line.rstrip()}')

print('\n=== Reading settings.vue ===')
try:
    settings = read_file('gui/app/pages/settings.vue')
    print(f'  settings.vue: {len(settings.splitlines())} lines')
    for i, line in enumerate(settings.splitlines()):
        if 'lineNumber' in line.lower() or 'vim' in line.lower() or 'editor' in line.lower() or 'crt' in line.lower() or 'glow' in line.lower():
            print(f'  {i+1}: {line.rstrip()}')
except:
    print('  Not found')

print('\n=== Reading SystemPromptEditor.vue ===')
try:
    spe = read_file('gui/app/components/SystemPromptEditor.vue')
    print(f'  SystemPromptEditor.vue: {len(spe.splitlines())} lines')
except:
    print('  Not found')

print('\n=== Reading TiptapMonacoBlock.vue ===')
try:
    tmb = read_file('gui/app/components/TiptapMonacoBlock.vue')
    print(f'  TiptapMonacoBlock.vue: {len(tmb.splitlines())} lines')
    for i, line in enumerate(tmb.splitlines()):
        if 'lineNumbers' in line or 'lineNumber' in line:
            print(f'  {i+1}: {line.rstrip()}')
except:
    print('  Not found')

print('\n=== Reading package.json deps ===')
pkg = json.loads(read_file('gui/package.json'))
deps = pkg.get('dependencies', {})
for k, v in sorted(deps.items()):
    if 'tiptap' in k or 'monaco' in k or 'tippy' in k:
        print(f'  {k}: {v}')

print('\n=== Reading SlashCommands.ts ===')
try:
    sc = read_file('gui/app/extensions/SlashCommands.ts')
    print(f'  SlashCommands.ts: {len(sc.splitlines())} lines')
except:
    print('  Not found')

print('\n=== Reading FloatingToolbar.vue ===')
try:
    ft = read_file('gui/app/components/FloatingToolbar.vue')
    print(f'  FloatingToolbar.vue: {len(ft.splitlines())} lines')
except:
    print('  Not found')

# Show lines 80-200 of ChatInput.vue (the script/setup section)
print('\n=== ChatInput.vue lines 80-200 ===')
for i in range(79, min(200, len(lines))):
    print(f'{i+1}: {lines[i]}')
