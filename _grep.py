import sys  
import codecs  
import re  
sys.stdout = codecs.getwriter('utf-8')(sys.stdout.buffer)  
pat = re.compile(sys.argv[2], re.IGNORECASE)  
with open(sys.argv[1], 'r', encoding='utf-8') as f:  
    for i, line in enumerate(f, 1):  
        if pat.search(line):  
            print(f'{i}: {line}', end='') 
