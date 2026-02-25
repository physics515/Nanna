import sys  
sys.stdout.reconfigure(encoding='utf-8')  
with open('ROADMAP.md',encoding='utf-8') as f:  
 content = f.read()  
print(content[40000:48000])  
