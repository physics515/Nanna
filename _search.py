import sys  
patterns = ['StreamEvent', 'input_tokens', 'output_tokens', 'ChatResult', 'cache_read', 'cache_creation']  
with open('crates/nanna-llm/src/lib.rs', 'r', encoding='utf-8') as f:  
    for i, line in enumerate(f, 1):  
        if any(p in line for p in patterns):  
            print(f'{i}: {line}', end='') 
