<template>
  <div class="space-y-4">
    <!-- Step Indicator -->
    <div class="flex items-center gap-2 mb-6">
      <div 
        v-for="s in 3" 
        :key="s"
        :class="[
          'w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium transition-colors',
          step >= s 
            ? 'bg-nanna-primary text-white' 
            : 'bg-nanna-bg-elevated text-nanna-text-dim'
        ]"
      >
        {{ s }}
      </div>
      <div class="flex-1" />
      <span class="text-xs text-nanna-text-dim">Step {{ step }} of 3</span>
    </div>
    
    <!-- Step 1: Create Bot -->
    <div v-if="step === 1" class="space-y-4">
      <h4 class="font-medium text-nanna-text">1. Create a Telegram Bot</h4>
      
      <div class="bg-nanna-bg-elevated/40 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          Open Telegram and search for <strong class="text-nanna-accent">@BotFather</strong>
        </p>
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Send <code class="bg-nanna-bg-deep px-1 rounded">/newbot</code> to BotFather</li>
          <li>Choose a name for your bot (e.g., "My Nanna")</li>
          <li>Choose a username ending in "bot" (e.g., "my_nanna_bot")</li>
          <li>BotFather will give you an API token</li>
        </ol>
        
        <a 
          href="https://t.me/BotFather" 
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          Open BotFather
        </a>
      </div>
      
      <UiButton @click="step = 2" class="w-full">
        I have my bot token
        <ArrowRight class="w-4 h-4 ml-2" />
      </UiButton>
    </div>
    
    <!-- Step 2: Enter Token -->
    <div v-if="step === 2" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. Enter Your Bot Token</h4>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Bot Token</label>
        <UiInput 
          v-model="botToken" 
          type="password"
          placeholder="123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Paste the token from BotFather (format: numbers:letters)
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 1" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 3" :disabled="!botToken" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 3: Configure & Test -->
    <div v-if="step === 3" class="space-y-4">
      <h4 class="font-medium text-nanna-text">3. Configure & Test</h4>
      
      <!-- Optional: Allowed Users -->
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">
          Allowed User IDs <span class="text-nanna-text-dim">(optional)</span>
        </label>
        <UiInput 
          v-model="allowedUsers" 
          placeholder="123456789, 987654321"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Comma-separated Telegram user IDs. Leave empty to allow anyone.
        </p>
      </div>
      
      <!-- Optional: Webhook URL -->
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">
          Webhook URL <span class="text-nanna-text-dim">(optional)</span>
        </label>
        <UiInput 
          v-model="webhookUrl" 
          placeholder="https://your-server.com/webhook/telegram"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          For production. Leave empty for polling mode (recommended for testing).
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 2" variant="secondary">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="testBot" variant="secondary" :disabled="testing">
          <UiSpinner v-if="testing" size="sm" class="mr-2" />
          <Zap v-else class="w-4 h-4 mr-2" />
          Test
        </UiButton>
        <UiButton @click="saveConfig" :disabled="!botToken || saving" class="flex-1">
          <UiSpinner v-if="saving" size="sm" class="mr-2" />
          <Save v-else class="w-4 h-4 mr-2" />
          Save Configuration
        </UiButton>
      </div>
      
      <!-- Test Result -->
      <div v-if="testResult" :class="[
        'p-3 rounded-lg text-sm',
        testResult.success ? 'bg-nanna-success/20 text-nanna-success' : 'bg-nanna-error/20 text-nanna-error'
      ]">
        {{ testResult.message }}
      </div>
    </div>
    
    <!-- Already Configured -->
    <div v-if="status?.configured" class="mt-4 pt-4 border-t border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <span class="w-2 h-2 rounded-full bg-nanna-success"></span>
          <span class="text-sm text-nanna-text">Currently connected</span>
        </div>
        <UiButton @click="$emit('test', 'Telegram')" variant="ghost" size="sm">
          Test Connection
        </UiButton>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { ExternalLink, ArrowRight, ArrowLeft, Zap, Save } from 'lucide-vue-next'

interface ChannelStatus {
  name: string
  configured: boolean
  enabled: boolean
  status: string
  details: string | null
}

const props = defineProps<{
  status?: ChannelStatus
}>()

const emit = defineEmits<{
  save: [channel: string, config: Record<string, string>]
  test: [channel: string]
}>()

const step = ref(props.status?.configured ? 3 : 1)
const botToken = ref('')
const allowedUsers = ref('')
const webhookUrl = ref('')
const testing = ref(false)
const saving = ref(false)
const testResult = ref<{ success: boolean; message: string } | null>(null)

async function testBot() {
  testing.value = true
  testResult.value = null
  
  try {
    // Simple validation test
    if (!botToken.value.includes(':')) {
      testResult.value = { success: false, message: 'Invalid token format. Should be numbers:letters' }
      return
    }
    
    // Try to call Telegram API
    const response = await fetch(`https://api.telegram.org/bot${botToken.value}/getMe`)
    const data = await response.json()
    
    if (data.ok) {
      testResult.value = { 
        success: true, 
        message: `✓ Connected to @${data.result.username} (${data.result.first_name})` 
      }
    } else {
      testResult.value = { success: false, message: `API Error: ${data.description}` }
    }
  } catch (e: any) {
    testResult.value = { success: false, message: `Connection failed: ${e.message}` }
  } finally {
    testing.value = false
  }
}

async function saveConfig() {
  saving.value = true
  
  const config: Record<string, string> = {
    bot_token: botToken.value,
  }
  
  if (webhookUrl.value) {
    config.webhook_url = webhookUrl.value
  }
  
  if (allowedUsers.value) {
    config.allowed_users = allowedUsers.value
  }
  
  emit('save', 'telegram', config)
  saving.value = false
}
</script>
