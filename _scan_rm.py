import sys
with open('ROADMAP.md', 'r', encoding='utf-8') as f:
    for i, line in enumerate(f, 1):
        s = line.strip()
        if s.startswith('#'):
            print(f'{i}: {s}')
