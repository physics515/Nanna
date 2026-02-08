import sys
with open('gui/src-tauri/src/lib.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()
start = int(sys.argv[1]) - 1
end = int(sys.argv[2])
for i in range(start, min(end, len(lines))):
    print(f'{i+1}: {lines[i]}', end='')
