with open(r'D:\Development\nanna\gui\src-tauri\src\lib.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

for i, line in enumerate(lines):
    if 'async fn send_message' in line:
        print(f"Found at line {i+1}")
        for j in range(max(0, i-5), min(i+300, len(lines))):
            print(f"{j+1}: {lines[j]}", end='')
        break
