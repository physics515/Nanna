import re

with open(r'D:\Development\nanna\gui\app\layouts\default.vue', 'r') as f:
    content = f.read()

logs_link = '''          <NuxtLink to="/logs" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FileText class="w-4 h-4" /><span>Logs</span>
          </NuxtLink>
'''

# Find Memory link and add Logs after it
pattern = r'(<NuxtLink to="/memory"[^<]*?</NuxtLink>)'
replacement = r'\1\n' + logs_link

content = re.sub(pattern, replacement, content)

with open(r'D:\Development\nanna\gui\app\layouts\default.vue', 'w') as f:
    f.write(content)

print("Updated default.vue with logs nav link")
