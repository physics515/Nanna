import sys, codecs
sys.stdout = codecs.getwriter('utf-8')(sys.stdout.buffer)
f = open('STATUS.md', 'r', encoding='utf-8')
lines = f.readlines()
f.close()
for i, line in enumerate(lines, 1):
 if any(k in line for k in ['Phase 7','Rich','Table','Vim','CRT','glow','Line number','localStorage','draft','System prompt','Memory editor','Workspace']):
  print(f'{i}: {line}', end=''  )
