import sys
import os
os.environ['PYTHONIOENCODING'] = 'utf-8'
sys.stdout.reconfigure(encoding='utf-8')
f = open(r'D:\Development\nanna\gui\src-tauri\src\lib.rs', 'r', encoding='utf-8')
lines = f.readlines()
f.close()
start = int(sys.argv[1]) - 1
end = int(sys.argv[2])
for i in range(start, min(end, len(lines))):
    print(f'{i+1}: {lines[i]}', end='')
