with open(r'D:\Development\nanna\gui\src-tauri\src\lib.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

# Find send_message with #[tauri::command]
for i, line in enumerate(lines):
    if 'async fn send_message' in line and i > 1200:  # After line 1200
        # Print 500 lines from here
        for j in range(max(0, i-3), min(i+500, len(lines))):
            print(f"{j+1}: {lines[j]}", end='')
        break
