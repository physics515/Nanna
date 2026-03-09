<template>
  <UiCard>
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-base font-semibold text-nanna-primary flex items-center gap-2">
        <MessageSquare class="w-4 h-4" />
        System Prompt
      </h3>
      <div class="flex items-center gap-2">
        <UiBadge v-if="hasChanges" variant="warning">Unsaved</UiBadge>
        <UiButton 
          v-if="!isEditing" 
          @click="startEditing" 
          variant="secondary" 
          size="sm"
        >
          <Edit3 class="w-3 h-3 mr-1" />
          Edit
        </UiButton>
      </div>
    </div>
    
    <!-- View Mode -->
    <div v-if="!isEditing" class="space-y-3">
      <div 
        class="bg-nanna-bg-elevated/40 rounded-lg p-4 max-h-64 overflow-y-auto"
      >
        <pre class="text-sm text-nanna-text whitespace-pre-wrap font-sans leading-relaxed">{{ currentPrompt || defaultPrompt }}</pre>
      </div>
      
      <div class="flex items-center justify-between text-xs text-nanna-text-dim">
        <span>{{ wordCount }} words · {{ charCount }} characters</span>
        <button 
          v-if="currentPrompt !== defaultPrompt" 
          @click="resetToDefault"
          class="text-nanna-accent hover:underline"
        >
          Reset to default
        </button>
      </div>
    </div>
    
    <!-- Edit Mode -->
    <div v-else class="space-y-4">
      <!-- Template Presets -->
      <div>
        <label class="block text-xs text-nanna-text-muted mb-2">Quick Templates</label>
        <div class="flex flex-wrap gap-2">
          <UiButton 
            v-for="template in templates" 
            :key="template.id"
            @click="applyTemplate(template)"
            variant="secondary"
            size="sm"
            class="text-xs"
          >
            {{ template.icon }} {{ template.name }}
          </UiButton>
        </div>
      </div>
      
      <!-- Editor -->
      <div class="relative">
        <textarea
          v-model="editedPrompt"
          :placeholder="defaultPrompt"
          class="w-full h-64 bg-nanna-bg-deep text-nanna-text text-sm p-4 rounded-lg border border-white/[0.06] focus:border-nanna-primary focus:outline-none resize-none font-mono leading-relaxed"
          @input="markChanged"
        />
        <div class="absolute bottom-2 right-2 text-xs text-nanna-text-dim">
          {{ editedWordCount }} words
        </div>
      </div>
      
      <!-- Variables Reference -->
      <details class="text-sm">
        <summary class="cursor-pointer text-nanna-text-muted hover:text-nanna-text">
          Available Variables
        </summary>
        <div class="mt-2 p-3 bg-nanna-bg-elevated/40 rounded-lg space-y-1 text-xs">
          <div v-for="v in variables" :key="v.name" class="flex gap-2">
            <code class="text-nanna-accent font-mono">{{ v.name }}</code>
            <span class="text-nanna-text-dim">{{ v.description }}</span>
          </div>
        </div>
      </details>
      
      <!-- Actions -->
      <div class="flex gap-2">
        <UiButton @click="cancelEditing" variant="secondary" class="flex-1">
          Cancel
        </UiButton>
        <UiButton @click="savePrompt" :disabled="saving || !hasChanges" class="flex-1">
          <UiSpinner v-if="saving" size="sm" class="mr-2" />
          <Save v-else class="w-4 h-4 mr-2" />
          Save
        </UiButton>
      </div>
    </div>
  </UiCard>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { MessageSquare, Edit3, Save } from 'lucide-vue-next'

const emit = defineEmits<{
  saved: []
  error: [message: string]
}>()

const defaultPrompt = `You are Nanna, a thoughtful AI assistant. You are helpful, harmless, and honest.

Be concise but thorough. Think step by step when solving complex problems.
If you don't know something, say so rather than making up information.
Match the user's communication style and tone.`

const templates = [
  { 
    id: 'default', 
    name: 'Default', 
    icon: '🌙',
    prompt: defaultPrompt
  },
  { 
    id: 'concise', 
    name: 'Concise', 
    icon: '⚡',
    prompt: `You are Nanna, a highly efficient AI assistant.
Be extremely concise. Prefer bullet points over paragraphs.
Skip pleasantries. Get straight to the point.
Only elaborate if explicitly asked.`
  },
  { 
    id: 'creative', 
    name: 'Creative', 
    icon: '🎨',
    prompt: `You are Nanna, a creative and imaginative AI assistant.
Think outside the box. Offer unique perspectives and creative solutions.
Use vivid language and analogies to explain concepts.
Don't be afraid to explore unconventional ideas.`
  },
  { 
    id: 'technical', 
    name: 'Technical', 
    icon: '🔧',
    prompt: `You are Nanna, a senior software engineer and technical expert.
Provide detailed, technically accurate responses.
Include code examples when relevant. Use proper terminology.
Consider edge cases and potential issues in your solutions.
Prefer modern best practices and idiomatic patterns.`
  },
  { 
    id: 'friendly', 
    name: 'Friendly', 
    icon: '💬',
    prompt: `You are Nanna, a warm and friendly AI companion.
Be conversational and personable. Use emojis occasionally.
Show genuine interest in helping. Celebrate successes with the user.
Make interactions feel natural and enjoyable.`
  },
]

const variables = [
  { name: '{{user_name}}', description: 'Current user\'s name' },
  { name: '{{date}}', description: 'Current date' },
  { name: '{{time}}', description: 'Current time' },
  { name: '{{os}}', description: 'Operating system' },
]

const currentPrompt = ref('')
const editedPrompt = ref('')
const isEditing = ref(false)
const hasChanges = ref(false)
const saving = ref(false)

const wordCount = computed(() => {
  const text = currentPrompt.value || defaultPrompt
  return text.trim().split(/\s+/).filter(Boolean).length
})

const charCount = computed(() => {
  return (currentPrompt.value || defaultPrompt).length
})

const editedWordCount = computed(() => {
  return editedPrompt.value.trim().split(/\s+/).filter(Boolean).length
})

onMounted(async () => {
  await loadPrompt()
})

async function loadPrompt() {
  try {
    const prompt = await invoke<string | null>('get_system_prompt')
    currentPrompt.value = prompt || ''
  } catch (e) {
    console.error('Failed to load system prompt:', e)
  }
}

function startEditing() {
  editedPrompt.value = currentPrompt.value || defaultPrompt
  isEditing.value = true
  hasChanges.value = false
}

function cancelEditing() {
  isEditing.value = false
  hasChanges.value = false
}

function markChanged() {
  hasChanges.value = editedPrompt.value !== (currentPrompt.value || defaultPrompt)
}

function applyTemplate(template: typeof templates[0]) {
  editedPrompt.value = template.prompt
  markChanged()
}

async function resetToDefault() {
  if (!confirm('Reset system prompt to default?')) return
  try {
    await invoke('set_system_prompt', { prompt: null })
    currentPrompt.value = ''
    emit('saved')
  } catch (e: any) {
    emit('error', e.message || 'Failed to reset')
  }
}

async function savePrompt() {
  saving.value = true
  try {
    // Save null if it's exactly the default, otherwise save the edited prompt
    const promptToSave = editedPrompt.value === defaultPrompt ? null : editedPrompt.value
    await invoke('set_system_prompt', { prompt: promptToSave })
    currentPrompt.value = editedPrompt.value === defaultPrompt ? '' : editedPrompt.value
    isEditing.value = false
    hasChanges.value = false
    emit('saved')
  } catch (e: any) {
    emit('error', e.message || 'Failed to save')
  } finally {
    saving.value = false
  }
}
</script>
