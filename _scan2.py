lines = open(\"gui/src-tauri/src/lib.rs\", encoding=\"utf-8\").readlines()  
for i, l in enumerate(lines):  
    s = l.strip()  
    if \"async fn \" in s and not s.startswith(\"//\"):  
        print(f\"{i+1}: {s[:120]}\")  
