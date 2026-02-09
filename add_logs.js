const fs = require('fs');
const path = require('path');

const filePath = path.join(__dirname, 'gui/app/layouts/default.vue');
let content = fs.readFileSync(filePath, 'utf8');

// The logs link to insert
const logsLink = `          <NuxtLink to="/logs" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FileText class="w-4 h-4" /><span>Logs</span>
          </NuxtLink>`;

// Find Memory link and insert Logs after it
// Pattern: Memory link closing tag followed by Workspaces link opening tag
const pattern = /(<NuxtLink to="\/memory"[^>]*>[\s\S]*?<\/NuxtLink>)\s+(<NuxtLink to="\/workspaces")/g;
const replacement = `$1\n${logsLink}\n          $2`;

content = content.replace(pattern, replacement);

// Make sure FileText is imported
if (!content.includes('FileText')) {
  // Find the import line and add FileText
  content = content.replace(
    /import \{([^}]*)\} from 'lucide-vue-next'/,
    (match, imports) => {
      if (!imports.includes('FileText')) {
        return `import { FileText, ${imports.trim()} } from 'lucide-vue-next'`;
      }
      return match;
    }
  );
}

fs.writeFileSync(filePath, content, 'utf8');
console.log('✓ Added logs nav link to default.vue');
