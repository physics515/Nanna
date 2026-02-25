import subprocess
result = subprocess.run(
    ["cargo", "check", "-p", "nanna-gpu"],
    capture_output=True, text=True,
    cwd=r"D:\Development\nanna"
)
# Print only lines containing "error"
for line in result.stderr.split('\n'):
    if 'error' in line.lower():
        print(line)
