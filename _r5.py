import sys
sys.stdout.reconfigure(encoding='utf-8')
with open('crates/nanna-storage/src/migrations.rs','r',encoding='utf-8') as f:
 lines=f.readlines()
for l in lines[260:]:
 print(l,end='')
