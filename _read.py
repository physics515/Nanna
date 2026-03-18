import sys  
import codecs  
sys.stdout = codecs.getwriter('utf-8')(sys.stdout.buffer)  
with open(sys.argv[1], 'r', encoding='utf-8') as f:  
    for i, line in enumerate(f, 1):  
        print(f'{i}: {line}', end='') 
