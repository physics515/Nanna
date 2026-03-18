f=open('crates/nanna-llm/src/lib.rs','r',encoding='utf-8')
lines=f.readlines()
for l in lines[748:775]:
    print(l,end=''')
