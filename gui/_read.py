import sys
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    lines = f.readlines()
start = int(sys.argv[2]) if len(sys.argv) > 2 else 1
end = int(sys.argv[3]) if len(sys.argv) > 3 else len(lines)
for i in range(max(0, start-1), min(end, len(lines))):
    print(f"{i+1}: {lines[i]}", end='')
