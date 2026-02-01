<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Tool Authoring</h2>
          <p class="text-xs sm:text-sm text-nanna-text-muted">
            Create and manage custom tools for Nanna
          </p>
        </div>
        <UiButton @click="openCreateModal" size="sm">
          ✨ Create Tool
        </UiButton>
      </div>
    </header>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <!-- Loading state -->
      <div v-if="loading" class="flex items-center justify-center min-h-[300px]">
        <div class="text-nanna-text-muted">Loading tools...</div>
      </div>

      <!-- Empty state -->
      <div v-else-if="tools.length === 0" class="flex items-center justify-center min-h-[300px]">
        <div class="text-center max-w-md px-4">
          <div class="text-5xl sm:text-6xl mb-4">🛠️</div>
          <h3 class="text-lg sm:text-xl font-semibold text-nanna-text mb-2">
            No Custom Tools Yet
          </h3>
          <p class="text-sm text-nanna-text-muted mb-6">
            Create JavaScript tools that Nanna can use during conversations.
            Tools can fetch data, perform calculations, or interact with external services.
          </p>
          <UiButton @click="openCreateModal">
            ✨ Create Your First Tool
          </UiButton>
        </div>
      </div>

      <!-- Tool list -->
      <div v-else class="space-y-3 sm:space-y-4">
        <UiCard
          v-for="tool in tools"
          :key="tool.name"
          class="p-4 hover:bg-nanna-bg-surface transition-colors"
        >
          <div class="flex items-start justify-between gap-4">
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2 mb-1">
                <span class="font-mono text-nanna-accent font-semibold">{{ tool.name }}</span>
                <UiBadge v-if="!tool.enabled" variant="warning" size="sm">disabled</UiBadge>
                <UiBadge variant="secondary" size="sm">{{ tool.language }}</UiBadge>
              </div>
              <p class="text-sm text-nanna-text-muted mb-2">{{ tool.description }}</p>
              <div class="text-xs text-nanna-text-dim">
                Created {{ formatDate(tool.created_at) }}
                <span v-if="tool.updated_at !== tool.created_at">
                  • Updated {{ formatDate(tool.updated_at) }}
                </span>
              </div>
            </div>
            <div class="flex items-center gap-2">
              <UiButton @click="editTool(tool)" variant="ghost" size="sm">
                ✏️ Edit
              </UiButton>
              <UiButton @click="testTool(tool)" variant="ghost" size="sm">
                ▶️ Test
              </UiButton>
              <UiButton @click="confirmDelete(tool)" variant="ghost" size="sm" class="text-red-400 hover:text-red-300">
                🗑️
              </UiButton>
            </div>
          </div>
        </UiCard>
      </div>
    </div>

    <!-- Create/Edit Modal -->
    <UiModal v-model="showModal" :title="editing ? 'Edit Tool' : 'Create Tool'" size="xl">
      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-sm font-medium text-nanna-text mb-1">Name</label>
          <UiInput
            v-model="form.name"
            placeholder="my_tool"
            :disabled="editing"
            class="font-mono"
          />
          <p class="text-xs text-nanna-text-dim mt-1">
            Lowercase with underscores. Cannot be changed after creation.
          </p>
        </div>

        <!-- Description -->
        <div>
          <label class="block text-sm font-medium text-nanna-text mb-1">Description</label>
          <UiInput
            v-model="form.description"
            placeholder="What does this tool do?"
          />
        </div>

        <!-- Source Code -->
        <div>
          <label class="block text-sm font-medium text-nanna-text mb-1">
            JavaScript Code
          </label>
          <div class="relative">
            <textarea
              v-model="form.source"
              class="w-full h-64 p-3 bg-nanna-bg-elevated border border-nanna-primary/20 rounded-lg font-mono text-sm text-nanna-text resize-none focus:outline-none focus:border-nanna-accent"
              placeholder="export default {
  name: 'my_tool',
  description: 'Does something useful',
  execute(params) {
    // Your code here
    return 'result';
  }
}"
            ></textarea>
          </div>
          <p class="text-xs text-nanna-text-dim mt-1">
            Export a default object with name, description, and execute function.
          </p>
        </div>

        <!-- Parameters Schema (optional) -->
        <div>
          <label class="block text-sm font-medium text-nanna-text mb-1">
            Parameters (JSON Schema, optional)
          </label>
          <textarea
            v-model="form.parametersJson"
            class="w-full h-32 p-3 bg-nanna-bg-elevated border border-nanna-primary/20 rounded-lg font-mono text-sm text-nanna-text resize-none focus:outline-none focus:border-nanna-accent"
            placeholder='{
  "properties": {
    "query": { "type": "string", "description": "Search query" }
  },
  "required": ["query"]
}'
          ></textarea>
        </div>

        <!-- Error -->
        <div v-if="error" class="p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 text-sm">
          {{ error }}
        </div>

        <!-- Actions -->
        <div class="flex justify-end gap-2 pt-4 border-t border-nanna-primary/10">
          <UiButton @click="showModal = false" variant="ghost">Cancel</UiButton>
          <UiButton @click="saveTool" :disabled="saving">
            {{ saving ? 'Saving...' : (editing ? 'Update' : 'Create') }}
          </UiButton>
        </div>
      </div>
    </UiModal>

    <!-- Test Modal -->
    <UiModal v-model="showTestModal" title="Test Tool" size="lg">
      <div class="space-y-4">
        <div>
          <label class="block text-sm font-medium text-nanna-text mb-1">
            Input Parameters (JSON)
          </label>
          <textarea
            v-model="testInput"
            class="w-full h-32 p-3 bg-nanna-bg-elevated border border-nanna-primary/20 rounded-lg font-mono text-sm text-nanna-text resize-none focus:outline-none focus:border-nanna-accent"
            placeholder='{ "param": "value" }'
          ></textarea>
        </div>

        <div v-if="testResult" class="p-4 bg-nanna-bg-elevated rounded-lg">
          <label class="block text-sm font-medium text-nanna-text mb-2">Result</label>
          <pre class="text-sm text-nanna-text-muted whitespace-pre-wrap font-mono">{{ testResult }}</pre>
        </div>

        <div v-if="testError" class="p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 text-sm">
          {{ testError }}
        </div>

        <div class="flex justify-end gap-2 pt-4 border-t border-nanna-primary/10">
          <UiButton @click="showTestModal = false" variant="ghost">Close</UiButton>
          <UiButton @click="runTest" :disabled="testing">
            {{ testing ? 'Running...' : '▶️ Run Test' }}
          </UiButton>
        </div>
      </div>
    </UiModal>

    <!-- Delete Confirmation -->
    <UiModal v-model="showDeleteModal" title="Delete Tool">
      <p class="text-nanna-text-muted mb-4">
        Are you sure you want to delete <strong class="text-nanna-text">{{ toolToDelete?.name }}</strong>?
        This cannot be undone.
      </p>
      <div class="flex justify-end gap-2">
        <UiButton @click="showDeleteModal = false" variant="ghost">Cancel</UiButton>
        <UiButton @click="deleteTool" variant="destructive" :disabled="deleting">
          {{ deleting ? 'Deleting...' : 'Delete' }}
        </UiButton>
      </div>
    </UiModal>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface UserTool {
  name: string
  description: string
  source: string
  language: string
  parameters: any
  permissions: any
  created_at: number
  updated_at: number
  enabled: boolean
}

const tools = ref<UserTool[]>([])
const loading = ref(true)
const error = ref('')

// Modal state
const showModal = ref(false)
const editing = ref(false)
const saving = ref(false)
const form = ref({
  name: '',
  description: '',
  source: '',
  parametersJson: '',
})

// Test modal state
const showTestModal = ref(false)
const testInput = ref('{}')
const testResult = ref('')
const testError = ref('')
const testing = ref(false)
const toolToTest = ref<UserTool | null>(null)

// Delete modal state
const showDeleteModal = ref(false)
const toolToDelete = ref<UserTool | null>(null)
const deleting = ref(false)

onMounted(async () => {
  await loadTools()
})

async function loadTools() {
  loading.value = true
  try {
    tools.value = await invoke('list_user_tools_cmd')
  } catch (e: any) {
    console.error('Failed to load tools:', e)
    error.value = e.toString()
  } finally {
    loading.value = false
  }
}

function openCreateModal() {
  editing.value = false
  form.value = {
    name: '',
    description: '',
    source: `export default {
  name: 'my_tool',
  description: 'Does something useful',
  execute(params) {
    // Your code here
    return 'Hello, world!';
  }
}`,
    parametersJson: '',
  }
  error.value = ''
  showModal.value = true
}

function editTool(tool: UserTool) {
  editing.value = true
  form.value = {
    name: tool.name,
    description: tool.description,
    source: tool.source,
    parametersJson: tool.parameters ? JSON.stringify(tool.parameters, null, 2) : '',
  }
  error.value = ''
  showModal.value = true
}

async function saveTool() {
  saving.value = true
  error.value = ''
  
  try {
    let parameters = null
    if (form.value.parametersJson.trim()) {
      try {
        parameters = JSON.parse(form.value.parametersJson)
      } catch {
        error.value = 'Invalid JSON in parameters schema'
        saving.value = false
        return
      }
    }

    if (editing.value) {
      await invoke('update_user_tool', {
        name: form.value.name,
        description: form.value.description,
        source: form.value.source,
        parameters,
      })
    } else {
      await invoke('create_user_tool', {
        name: form.value.name,
        description: form.value.description,
        source: form.value.source,
        language: 'javascript',
        parameters,
      })
    }
    
    showModal.value = false
    await loadTools()
  } catch (e: any) {
    error.value = e.toString()
  } finally {
    saving.value = false
  }
}

function testTool(tool: UserTool) {
  toolToTest.value = tool
  testInput.value = '{}'
  testResult.value = ''
  testError.value = ''
  showTestModal.value = true
}

async function runTest() {
  if (!toolToTest.value) return
  
  testing.value = true
  testResult.value = ''
  testError.value = ''
  
  try {
    let input = {}
    try {
      input = JSON.parse(testInput.value)
    } catch {
      testError.value = 'Invalid JSON input'
      testing.value = false
      return
    }
    
    testResult.value = await invoke('test_user_tool', {
      source: toolToTest.value.source,
      input,
    })
  } catch (e: any) {
    testError.value = e.toString()
  } finally {
    testing.value = false
  }
}

function confirmDelete(tool: UserTool) {
  toolToDelete.value = tool
  showDeleteModal.value = true
}

async function deleteTool() {
  if (!toolToDelete.value) return
  
  deleting.value = true
  try {
    await invoke('delete_user_tool', { name: toolToDelete.value.name })
    showDeleteModal.value = false
    await loadTools()
  } catch (e: any) {
    console.error('Failed to delete tool:', e)
  } finally {
    deleting.value = false
  }
}

function formatDate(timestamp: number) {
  return new Date(timestamp * 1000).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  })
}
</script>
