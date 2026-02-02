<template>
  <div class="flex h-full">
    <!-- Sidebar: Skills List -->
    <aside class="w-64 border-r border-nanna-primary/10 bg-nanna-bg-surface/50 flex flex-col">
      <header class="px-3 py-3 border-b border-nanna-primary/10">
        <div class="flex items-center justify-between mb-2">
          <h2 class="font-semibold text-nanna-text text-sm">Skills</h2>
          <div class="flex gap-1">
            <button @click="refreshSkills" class="p-1 rounded hover:bg-nanna-primary/20 text-nanna-text-muted hover:text-nanna-text transition-colors" title="Refresh">
              <RefreshCw class="w-4 h-4" :class="{ 'animate-spin': refreshing }" />
            </button>
            <button @click="openCreateModal" class="p-1 rounded hover:bg-nanna-primary/20 text-nanna-text-muted hover:text-nanna-text transition-colors" title="New Skill">
              <Plus class="w-4 h-4" />
            </button>
          </div>
        </div>
        <UiInput v-model="searchQuery" placeholder="Search skills..." size="sm" class="text-xs" />
      </header>

      <div class="flex-1 overflow-y-auto p-2">
        <!-- Loading -->
        <div v-if="loading" class="flex items-center justify-center py-8">
          <Loader2 class="w-5 h-5 animate-spin text-nanna-text-muted" />
        </div>

        <!-- Skills list -->
        <div v-else class="space-y-1">
          <div
            v-for="skill in filteredSkills"
            :key="skill.name"
            @click="selectSkill(skill)"
            class="group flex items-center gap-2 px-2 py-1.5 rounded cursor-pointer transition-colors"
            :class="selectedSkill?.name === skill.name 
              ? 'bg-nanna-accent/20 text-nanna-accent' 
              : 'hover:bg-nanna-primary/10 text-nanna-text-muted hover:text-nanna-text'"
          >
            <component :is="getSkillIcon(skill)" class="w-4 h-4 flex-shrink-0" />
            <span class="text-sm truncate flex-1">{{ skill.name }}</span>
            <UiBadge v-if="skill.type === 'manifest'" variant="secondary" size="xs">yaml</UiBadge>
            <UiBadge v-else-if="skill.type === 'script'" variant="secondary" size="xs">{{ skill.language }}</UiBadge>
          </div>

          <!-- Empty state -->
          <div v-if="filteredSkills.length === 0 && !loading" class="text-center py-8 text-nanna-text-muted text-sm">
            <FileCode2 class="w-8 h-8 mx-auto mb-2 opacity-50" />
            <p>No skills found</p>
            <button @click="openCreateModal" class="text-nanna-accent hover:underline mt-2 text-xs">
              Create your first skill
            </button>
          </div>
        </div>
      </div>

      <!-- Workspace path -->
      <footer class="px-3 py-2 border-t border-nanna-primary/10 text-[10px] text-nanna-text-dim truncate">
        📁 {{ skillsPath || 'No workspace' }}
      </footer>
    </aside>

    <!-- Main Content -->
    <main class="flex-1 flex flex-col min-w-0">
      <!-- No skill selected -->
      <div v-if="!selectedSkill && !creating" class="flex-1 flex items-center justify-center">
        <div class="text-center max-w-md px-8">
          <Wrench class="w-16 h-16 mx-auto mb-4 text-nanna-text-dim" />
          <h3 class="text-lg font-semibold text-nanna-text mb-2">Tool Authoring</h3>
          <p class="text-sm text-nanna-text-muted mb-6">
            Create custom tools that Nanna can use during conversations.
            Select a skill from the sidebar or create a new one.
          </p>
          <div class="flex gap-2 justify-center">
            <UiButton @click="openCreateModal" size="sm">
              <Plus class="w-4 h-4 mr-1" /> New Skill
            </UiButton>
            <UiButton @click="openTemplates" variant="ghost" size="sm">
              <FileCode2 class="w-4 h-4 mr-1" /> Templates
            </UiButton>
          </div>
        </div>
      </div>

      <!-- Skill Editor -->
      <div v-else class="flex-1 flex flex-col min-h-0">
        <!-- Editor Header -->
        <header class="px-4 py-2 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <div class="flex items-center gap-2">
                <component :is="getSkillIcon(selectedSkill || editingSkill)" class="w-5 h-5 text-nanna-accent" />
                <span class="font-mono font-semibold text-nanna-text">
                  {{ creating ? 'New Skill' : selectedSkill?.name }}
                </span>
              </div>
              <UiBadge v-if="hasChanges" variant="warning" size="sm">unsaved</UiBadge>
            </div>
            <div class="flex items-center gap-2">
              <UiButton @click="testSkill" variant="ghost" size="sm" :disabled="!canTest">
                <Play class="w-4 h-4 mr-1" /> Test
              </UiButton>
              <UiButton @click="saveSkill" size="sm" :disabled="!hasChanges || saving">
                <Save class="w-4 h-4 mr-1" />
                {{ saving ? 'Saving...' : 'Save' }}
              </UiButton>
              <UiButton v-if="selectedSkill && !creating" @click="confirmDelete" variant="ghost" size="sm" class="text-red-400 hover:text-red-300">
                <Trash2 class="w-4 h-4" />
              </UiButton>
            </div>
          </div>
        </header>

        <!-- Editor Tabs -->
        <div class="flex border-b border-nanna-primary/10 bg-nanna-bg-surface/30">
          <button
            v-for="tab in editorTabs"
            :key="tab.id"
            @click="activeTab = tab.id"
            class="px-4 py-2 text-sm font-medium transition-colors border-b-2"
            :class="activeTab === tab.id 
              ? 'text-nanna-accent border-nanna-accent' 
              : 'text-nanna-text-muted hover:text-nanna-text border-transparent'"
          >
            {{ tab.label }}
          </button>
        </div>

        <!-- Editor Content -->
        <div class="flex-1 flex min-h-0">
          <!-- Code Editor Panel -->
          <div class="flex-1 flex flex-col min-w-0" v-show="activeTab === 'code'">
            <!-- Creating: Name input -->
            <div v-if="creating" class="px-4 py-3 border-b border-nanna-primary/10 bg-nanna-bg-surface/30">
              <div class="flex gap-4">
                <div class="flex-1">
                  <label class="block text-xs font-medium text-nanna-text-muted mb-1">Skill Name</label>
                  <UiInput v-model="editingSkill.name" placeholder="my_tool" class="font-mono text-sm" />
                </div>
                <div class="w-32">
                  <label class="block text-xs font-medium text-nanna-text-muted mb-1">Type</label>
                  <select v-model="editingSkill.type" class="w-full px-3 py-2 bg-nanna-bg-elevated border border-nanna-primary/20 rounded-lg text-sm text-nanna-text">
                    <option value="script">Script (JS/TS)</option>
                    <option value="manifest">Manifest (YAML)</option>
                  </select>
                </div>
              </div>
            </div>

            <!-- Monaco Editor -->
            <div class="flex-1 min-h-0">
              <VueMonacoEditor
                v-model:value="editingSkill.code"
                :language="editorLanguage"
                theme="nanna-dark"
                :options="editorOptions"
                @mount="onEditorMount"
                class="h-full"
              />
            </div>
          </div>

          <!-- Schema Tab -->
          <div v-show="activeTab === 'schema'" class="flex-1 p-4 overflow-y-auto">
            <div class="max-w-2xl">
              <h3 class="text-sm font-semibold text-nanna-text mb-2">Parameters Schema</h3>
              <p class="text-xs text-nanna-text-muted mb-4">
                Define the JSON Schema for your tool's parameters. This helps Nanna understand what inputs your tool expects.
              </p>
              <div class="h-64">
                <VueMonacoEditor
                  v-model:value="editingSkill.parametersJson"
                  language="json"
                  theme="nanna-dark"
                  :options="{ ...editorOptions, lineNumbers: 'on' }"
                />
              </div>
            </div>
          </div>

          <!-- Preview Tab -->
          <div v-show="activeTab === 'preview'" class="flex-1 p-4 overflow-y-auto">
            <div class="max-w-2xl space-y-4">
              <div>
                <h3 class="text-sm font-semibold text-nanna-text mb-2">Tool Preview</h3>
                <p class="text-xs text-nanna-text-muted">
                  How Nanna sees this tool
                </p>
              </div>
              
              <UiCard class="p-4">
                <div class="flex items-start gap-3">
                  <div class="w-10 h-10 rounded-lg bg-nanna-accent/20 flex items-center justify-center">
                    <Wrench class="w-5 h-5 text-nanna-accent" />
                  </div>
                  <div class="flex-1">
                    <div class="font-mono font-semibold text-nanna-text">{{ parsedTool.name || 'unnamed' }}</div>
                    <div class="text-sm text-nanna-text-muted mt-1">{{ parsedTool.description || 'No description' }}</div>
                  </div>
                </div>

                <div v-if="parsedTool.parameters" class="mt-4 pt-4 border-t border-nanna-primary/10">
                  <div class="text-xs font-medium text-nanna-text-muted mb-2">Parameters</div>
                  <div class="space-y-2">
                    <div v-for="(param, key) in parsedTool.parameters.properties || {}" :key="key" class="flex items-center gap-2 text-sm">
                      <code class="px-1.5 py-0.5 bg-nanna-bg-elevated rounded text-nanna-accent text-xs">{{ key }}</code>
                      <span class="text-nanna-text-dim">{{ param.type }}</span>
                      <span v-if="parsedTool.parameters.required?.includes(key)" class="text-red-400 text-xs">required</span>
                      <span v-if="param.description" class="text-nanna-text-muted text-xs">— {{ param.description }}</span>
                    </div>
                  </div>
                </div>
              </UiCard>

              <!-- Validation -->
              <div v-if="validationErrors.length > 0" class="p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
                <div class="text-sm font-medium text-red-400 mb-1">Validation Errors</div>
                <ul class="text-xs text-red-300 space-y-1">
                  <li v-for="(err, i) in validationErrors" :key="i">• {{ err }}</li>
                </ul>
              </div>
            </div>
          </div>

          <!-- Test Panel (slide out) -->
          <aside v-if="showTestPanel" class="w-80 border-l border-nanna-primary/10 bg-nanna-bg-surface/50 flex flex-col">
            <header class="px-3 py-2 border-b border-nanna-primary/10 flex items-center justify-between">
              <span class="text-sm font-semibold text-nanna-text">Test Runner</span>
              <button @click="showTestPanel = false" class="p-1 rounded hover:bg-nanna-primary/20">
                <X class="w-4 h-4 text-nanna-text-muted" />
              </button>
            </header>

            <div class="flex-1 overflow-y-auto p-3 space-y-4">
              <!-- Auto-generated form from schema -->
              <div v-if="parsedTool.parameters?.properties">
                <div class="text-xs font-medium text-nanna-text-muted mb-2">Input Parameters</div>
                <div class="space-y-3">
                  <div v-for="(param, key) in parsedTool.parameters.properties" :key="key">
                    <label class="block text-xs text-nanna-text mb-1">
                      {{ key }}
                      <span v-if="parsedTool.parameters.required?.includes(key)" class="text-red-400">*</span>
                    </label>
                    <UiInput
                      v-model="testInputs[key]"
                      :placeholder="param.description || param.type"
                      size="sm"
                      class="text-xs"
                    />
                  </div>
                </div>
              </div>

              <!-- Raw JSON input -->
              <div v-else>
                <label class="block text-xs font-medium text-nanna-text-muted mb-1">Input (JSON)</label>
                <textarea
                  v-model="testInputJson"
                  class="w-full h-24 p-2 bg-nanna-bg-elevated border border-nanna-primary/20 rounded text-xs font-mono text-nanna-text resize-none"
                  placeholder='{ "param": "value" }'
                ></textarea>
              </div>

              <UiButton @click="runTest" :disabled="testing" class="w-full" size="sm">
                <Play class="w-4 h-4 mr-1" />
                {{ testing ? 'Running...' : 'Run Test' }}
              </UiButton>

              <!-- Test Result -->
              <div v-if="testResult !== null" class="space-y-2">
                <div class="text-xs font-medium text-nanna-text-muted">Result</div>
                <div class="p-3 bg-nanna-bg-elevated rounded text-xs font-mono whitespace-pre-wrap" :class="testError ? 'text-red-400' : 'text-green-400'">
                  {{ testResult }}
                </div>
                <div v-if="testDuration" class="text-[10px] text-nanna-text-dim">
                  Completed in {{ testDuration }}ms
                </div>
              </div>
            </div>
          </aside>
        </div>
      </div>
    </main>

    <!-- Create Skill Modal -->
    <UiModal v-model="showCreateModal" title="Create New Skill" size="lg">
      <div class="space-y-4">
        <p class="text-sm text-nanna-text-muted">
          Choose a template to get started:
        </p>

        <div class="grid grid-cols-2 gap-3">
          <button
            v-for="template in templates"
            :key="template.id"
            @click="createFromTemplate(template)"
            class="p-4 text-left bg-nanna-bg-elevated hover:bg-nanna-primary/10 border border-nanna-primary/20 hover:border-nanna-accent/50 rounded-lg transition-colors"
          >
            <div class="text-lg mb-1">{{ template.icon }}</div>
            <div class="font-medium text-nanna-text text-sm">{{ template.name }}</div>
            <div class="text-xs text-nanna-text-muted mt-1">{{ template.description }}</div>
          </button>
        </div>

        <div class="flex justify-end pt-4 border-t border-nanna-primary/10">
          <UiButton @click="showCreateModal = false" variant="ghost">Cancel</UiButton>
        </div>
      </div>
    </UiModal>

    <!-- Delete Confirmation -->
    <UiModal v-model="showDeleteModal" title="Delete Skill">
      <p class="text-nanna-text-muted mb-4">
        Are you sure you want to delete <strong class="text-nanna-text">{{ selectedSkill?.name }}</strong>?
        This will remove all files in the skill folder.
      </p>
      <div class="flex justify-end gap-2">
        <UiButton @click="showDeleteModal = false" variant="ghost">Cancel</UiButton>
        <UiButton @click="deleteSkill" variant="destructive" :disabled="deleting">
          {{ deleting ? 'Deleting...' : 'Delete' }}
        </UiButton>
      </div>
    </UiModal>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { VueMonacoEditor } from '@guolao/vue-monaco-editor'
import { 
  Plus, RefreshCw, Loader2, FileCode2, Wrench, Play, Save, Trash2, X,
  FileJson, FileType, Terminal, Globe, Code2
} from 'lucide-vue-next'

interface Skill {
  name: string
  type: 'script' | 'manifest'
  language?: string
  path: string
  code?: string
  manifest?: any
}

interface Template {
  id: string
  name: string
  icon: string
  description: string
  type: 'script' | 'manifest'
  code: string
}

// State
const loading = ref(true)
const refreshing = ref(false)
const saving = ref(false)
const deleting = ref(false)
const testing = ref(false)
const searchQuery = ref('')
const skills = ref<Skill[]>([])
const skillsPath = ref('')
const selectedSkill = ref<Skill | null>(null)
const creating = ref(false)
const hasChanges = ref(false)
const showTestPanel = ref(false)
const showCreateModal = ref(false)
const showDeleteModal = ref(false)
const activeTab = ref<'code' | 'schema' | 'preview'>('code')

// Editing state
const editingSkill = ref({
  name: '',
  type: 'script' as 'script' | 'manifest',
  code: '',
  parametersJson: '{}',
})
const originalCode = ref('')

// Test state
const testInputs = ref<Record<string, string>>({})
const testInputJson = ref('{}')
const testResult = ref<string | null>(null)
const testError = ref(false)
const testDuration = ref<number | null>(null)

// Templates
const templates: Template[] = [
  {
    id: 'blank-ts',
    name: 'Blank TypeScript',
    icon: '📝',
    description: 'Empty tool template',
    type: 'script',
    code: `export default {
  name: 'my_tool',
  description: 'Describe what your tool does',
  parameters: {
    type: 'object',
    properties: {
      input: { type: 'string', description: 'Input value' }
    },
    required: ['input']
  },
  execute(params) {
    return \`Result: \${params.input}\`;
  }
}`
  },
  {
    id: 'api-fetch',
    name: 'API Fetch',
    icon: '🌐',
    description: 'Fetch data from an API',
    type: 'script',
    code: `export default {
  name: 'api_fetch',
  description: 'Fetch data from an API endpoint',
  parameters: {
    type: 'object',
    properties: {
      url: { type: 'string', description: 'API endpoint URL' },
      method: { type: 'string', enum: ['GET', 'POST'], default: 'GET' }
    },
    required: ['url']
  },
  async execute(params) {
    const response = await fetch(params.url, {
      method: params.method || 'GET'
    });
    return await response.json();
  }
}`
  },
  {
    id: 'file-processor',
    name: 'File Processor',
    icon: '📄',
    description: 'Read and process files',
    type: 'manifest',
    code: `name: file_processor
description: Process a file with a shell script

parameters:
  type: object
  properties:
    file:
      type: string
      description: Path to the input file
    operation:
      type: string
      enum: [count_lines, word_count, checksum]
  required: [file, operation]

shell: |
  case "{{operation}}" in
    count_lines) wc -l "{{file}}" ;;
    word_count) wc -w "{{file}}" ;;
    checksum) sha256sum "{{file}}" ;;
  esac`
  },
  {
    id: 'python-script',
    name: 'Python Script',
    icon: '🐍',
    description: 'Run Python code',
    type: 'manifest',
    code: `name: python_tool
description: Execute a Python script

parameters:
  type: object
  properties:
    data:
      type: string
      description: Input data to process
  required: [data]

python: |
  import json
  import sys
  
  data = sys.argv[1] if len(sys.argv) > 1 else ''
  
  # Your Python code here
  result = f"Processed: {data}"
  print(result)`
  }
]

// Computed
const filteredSkills = computed(() => {
  const query = searchQuery.value.toLowerCase()
  return skills.value.filter(s => s.name.toLowerCase().includes(query))
})

const editorTabs = computed(() => [
  { id: 'code', label: editingSkill.value.type === 'manifest' ? 'Manifest' : 'Code' },
  { id: 'schema', label: 'Schema' },
  { id: 'preview', label: 'Preview' },
])

const editorLanguage = computed(() => {
  if (editingSkill.value.type === 'manifest') return 'yaml'
  return 'typescript'
})

const editorOptions = computed(() => ({
  minimap: { enabled: false },
  lineNumbers: 'on',
  fontSize: 13,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontLigatures: true,
  scrollBeyondLastLine: false,
  wordWrap: 'on',
  tabSize: 2,
  automaticLayout: true,
}))

const canTest = computed(() => {
  return editingSkill.value.code.trim().length > 0
})

const parsedTool = computed(() => {
  try {
    if (editingSkill.value.type === 'manifest') {
      // Simple YAML-like parsing for preview
      const lines = editingSkill.value.code.split('\n')
      const result: any = {}
      for (const line of lines) {
        const match = line.match(/^(\w+):\s*(.*)$/)
        if (match) {
          result[match[1]] = match[2] || undefined
        }
      }
      return result
    } else {
      // Try to extract from JS exports
      const nameMatch = editingSkill.value.code.match(/name:\s*['"]([^'"]+)['"]/)
      const descMatch = editingSkill.value.code.match(/description:\s*['"]([^'"]+)['"]/)
      const paramsMatch = editingSkill.value.code.match(/parameters:\s*(\{[\s\S]*?\n\s*\})/)
      
      return {
        name: nameMatch?.[1] || '',
        description: descMatch?.[1] || '',
        parameters: paramsMatch ? tryParseJson(paramsMatch[1]) : null
      }
    }
  } catch {
    return { name: '', description: '', parameters: null }
  }
})

const validationErrors = computed(() => {
  const errors: string[] = []
  if (!parsedTool.value.name) errors.push('Missing tool name')
  if (!parsedTool.value.description) errors.push('Missing description')
  return errors
})

// Methods
function getSkillIcon(skill: Skill | null | undefined) {
  if (!skill) return Code2
  if (skill.type === 'manifest') return FileJson
  if (skill.language === 'python') return Terminal
  return FileType
}

function tryParseJson(str: string) {
  try {
    return JSON.parse(str.replace(/'/g, '"'))
  } catch {
    return null
  }
}

async function refreshSkills() {
  refreshing.value = true
  try {
    const result = await invoke<{ skills: Skill[], path: string }>('list_skills')
    skills.value = result.skills
    skillsPath.value = result.path
  } catch (e) {
    console.error('Failed to load skills:', e)
  } finally {
    refreshing.value = false
    loading.value = false
  }
}

function selectSkill(skill: Skill) {
  if (hasChanges.value && !confirm('Discard unsaved changes?')) return
  
  selectedSkill.value = skill
  creating.value = false
  editingSkill.value = {
    name: skill.name,
    type: skill.type,
    code: skill.code || '',
    parametersJson: '{}',
  }
  originalCode.value = skill.code || ''
  hasChanges.value = false
  activeTab.value = 'code'
}

function openCreateModal() {
  showCreateModal.value = true
}

function openTemplates() {
  showCreateModal.value = true
}

function createFromTemplate(template: Template) {
  showCreateModal.value = false
  selectedSkill.value = null
  creating.value = true
  editingSkill.value = {
    name: '',
    type: template.type,
    code: template.code,
    parametersJson: '{}',
  }
  originalCode.value = ''
  hasChanges.value = true
  activeTab.value = 'code'
}

async function saveSkill() {
  saving.value = true
  try {
    if (creating.value) {
      await invoke('create_skill', {
        name: editingSkill.value.name,
        skillType: editingSkill.value.type,
        code: editingSkill.value.code,
      })
      creating.value = false
    } else {
      await invoke('update_skill', {
        name: editingSkill.value.name,
        code: editingSkill.value.code,
      })
    }
    originalCode.value = editingSkill.value.code
    hasChanges.value = false
    await refreshSkills()
    
    // Re-select the skill
    const updated = skills.value.find(s => s.name === editingSkill.value.name)
    if (updated) selectSkill(updated)
  } catch (e: any) {
    alert('Failed to save: ' + e.toString())
  } finally {
    saving.value = false
  }
}

function confirmDelete() {
  showDeleteModal.value = true
}

async function deleteSkill() {
  if (!selectedSkill.value) return
  deleting.value = true
  try {
    await invoke('delete_skill', { name: selectedSkill.value.name })
    selectedSkill.value = null
    creating.value = false
    await refreshSkills()
  } catch (e: any) {
    alert('Failed to delete: ' + e.toString())
  } finally {
    deleting.value = false
    showDeleteModal.value = false
  }
}

function testSkill() {
  showTestPanel.value = true
  testResult.value = null
  testInputs.value = {}
  testInputJson.value = '{}'
}

async function runTest() {
  testing.value = true
  testError.value = false
  testResult.value = null
  testDuration.value = null
  
  const start = Date.now()
  try {
    let input = {}
    if (parsedTool.value.parameters?.properties) {
      input = { ...testInputs.value }
    } else {
      input = JSON.parse(testInputJson.value)
    }
    
    testResult.value = await invoke<string>('test_skill', {
      code: editingSkill.value.code,
      skillType: editingSkill.value.type,
      input,
    })
  } catch (e: any) {
    testError.value = true
    testResult.value = e.toString()
  } finally {
    testDuration.value = Date.now() - start
    testing.value = false
  }
}

function onEditorMount(editor: any) {
  // Editor mounted
}

// Watch for changes
watch(() => editingSkill.value.code, (newCode) => {
  hasChanges.value = newCode !== originalCode.value
})

// Load on mount
onMounted(() => {
  refreshSkills()
})
</script>
