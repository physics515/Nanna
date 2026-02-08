with open('gui/src-tauri/src/lib.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()
for i in range(4809, min(4960, len(lines))):
    print(f'{i+1}: {lines[i]}', end='')
