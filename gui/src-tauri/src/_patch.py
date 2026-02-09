import os

path = r'D:\Development\nanna\gui\src-tauri\src\lib.rs'
with open(path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

print(f'Total lines: {len(lines)}')
# Verify lines (1-indexed: 927, 928, 7851, 7852)
for i in [926, 927, 7850, 7851]:
    print(f'Line {i+1}: {lines[i].rstrip()}')
