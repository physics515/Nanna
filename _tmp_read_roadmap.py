import sys  
sys.stdout.reconfigure(encoding='utf-8')  
with open('ROADMAP.md', 'r', encoding='utf-8') as f:  
    content = f.read()  
    start = int(sys.argv[1]) if len(sys.argv) > 1 else 0  
    end = int(sys.argv[2]) if len(sys.argv) > 2 else 8000  
    print(content[start:end])  
