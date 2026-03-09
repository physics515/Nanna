import sys  
sys.stdout.reconfigure(encoding='utf-8')  
with open('ROADMAP.md','r',encoding='utf-8') as f:  
    lines = f.readlines()  
for i, line in enumerate(lines):  
    if line.startswith('#'):  
        print(f'{i+1}: {line.rstrip()}')  
