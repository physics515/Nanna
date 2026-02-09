#!/usr/bin/env python3
import re

filepath = r'D:\Development\nanna\gui\app\layouts\default.vue'

with open(filepath, 'r', encoding='utf-8') as f:
    content = f.read()

# The logs link to insert
logs_link = '''          <NuxtLink to="/logs" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FileText class="w-4 h-4" /><span>Logs</span>
          </NuxtLink>
'''

# Find Memory link closing and insert Logs link after it
# Pattern: </NuxtLink> followed by whitespace and <NuxtLink to="/workspaces"
pattern = r'(</NuxtLink>)(\s+)(<NuxtLink to="/workspaces")'
replacement = r'\1\2' + logs_link + r'\2\3'

# Replace all occurrences (both mobile and desktop sections)
content = re.sub(pattern, replacement, content)

# Ensure FileText is imported
if 'FileText' not in content:
    # Find the lucide import and add FileText
    content = re.sub(
        r'(from "lucide-vue-next"[^}]*?)(Brain)',
        r'\1FileText, \2',
        content
    )

with open(filepath, 'w', encoding='utf-8') as f:
    f.write(content)

print("✓ Added Logs nav link to default.vue")
print("✓ Ensured FileText icon is imported")
