import sys
f = open('ROADMAP.md', 'r', encoding='utf-8')
lines = f.readlines()
f.close()
for i, l in enumerate(lines):
    s = l.strip()
    if s.startswith('#'):
        print(str(i+1) + ': ' + s)
