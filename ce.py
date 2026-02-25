import os  
f = open(r'crates\nanna-gpu\src\memory_manager.rs', encoding='utf-8')  
lines = f.readlines()  
print(''.join(lines[290:340]))  
