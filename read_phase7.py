f = open('ROADMAP.md', 'r', encoding='utf-8')
lines = f.readlines()
for i, l in enumerate(lines):
    if '- [' in l and 210 < i < 310:
        print(f'{i+1}: {l.rstrip()}')
