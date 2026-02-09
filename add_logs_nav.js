const fs = require('fs');
const path = require('path');

const filePath = path.join(__dirname, 'gui/app/layouts/default.vue');
let content = fs.readFileSync(filePath, 'utf-8');

// Add FileText to imports if not present
if (!content.includes('FileText')) {
  content = content.replace(
    /from 'lucide-vue-next'/,
    "from 'lucide-vue-next'"
  );
  // Find the imports line and add FileText
  content = content.replace(
    /(import.*?)(Menu|Plus|Brain|FolderKanban|Bot|Radio|Wrench|Clock|Settings|LogOut)/,
    '$1FileText, $2'
  );
}

// Add logs link after memory link
const logsLink = `          </NuxtLink>
          <NuxtLink to="/logs" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FileText class="w-4 h-4" /><span>Logs</span>`;

// Replace both mobile and desktop occurrences
content = content.replace(
  /(<NuxtLink to="\/memory"[^>]*>[^<]*<Brain class="w-4 h-4"[^>]*><span>Memory<\/span>\s*<\/NuxtLink>)/g,
  '$1\n' + logsLink
);

fs.writeFileSync(filePath, content, 'utf-8');
console.log('✓ Added logs nav link to default.vue');
