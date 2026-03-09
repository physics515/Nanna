import sys  
sys.stdout.reconfigure(encoding='utf-8')  
with open('ROADMAP.md','r',encoding='utf-8') as f:  
    data = f.read()  
print(data[48000:])  
