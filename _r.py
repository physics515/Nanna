import sys 
sys.stdout.reconfigure(encoding='utf-8') 
with open('planning/10-warning-fixes.md',encoding='utf-8') as f: 
 for l in f: 
  s=l.strip() 
  if s.startswith('#') or s.startswith('- ['): print(l.rstrip())
