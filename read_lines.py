import sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
f = sys.argv[1]
s, e = int(sys.argv[2]), int(sys.argv[3])
lines = open(f, encoding='utf-8').readlines()
for i in range(s-1, min(e, len(lines))):
    print(f"{i+1}: {lines[i]}", end='')
