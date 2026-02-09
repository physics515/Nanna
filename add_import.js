const fs = require('fs');
const path = 'gui/app/layouts/default.vue';
let content = fs.readFileSync(path, 'utf8');

// Add FileText to the import line
content = content.replace(
  /import { (Menu, Plus, Brain, Radio, Settings, ChevronDown, FolderKanban, Bot, Wrench, Clock, Globe) } from 'lucide-vue-next'/,
  "import { $1, FileText } from 'lucide-vue-next'"
);

fs.writeFileSync(path, content);
console.log('✓ Added FileText import');
