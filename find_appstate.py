import re

path = r"D:\Development\nanna\gui\src-tauri\src\lib.rs"
with open(path, 'r') as f:
    lines = f.readlines()

for i, line in enumerate(lines):
    if 'pub struct AppState' in line:
        print(f"Found at line {i+1}")
        # Print next 50 lines
        for j in range(i, min(i+50, len(lines))):
            print(f"{j+1:4d}: {lines[j]}", end='')
        break
