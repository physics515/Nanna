<template>
  <div class="space-y-4">
    <!-- Step Indicator -->
    <div class="flex items-center gap-2 mb-6">
      <div 
        v-for="s in 4" 
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
      <span class="text-xs text-nanna-text-dim">Step {{ step }} of 4</span>
    </div>
    
    <!-- Step 1: Create Slack App -->
    <div v-if="step === 1" class="space-y-4">
      <h4 class="font-medium text-nanna-text">1. Create a Slack App</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Go to the Slack API portal</li>
          <li>Click <strong>"Create New App"</strong> → <strong>"From scratch"</strong></li>
          <li>Name your app (e.g., "Nanna Bot")</li>
          <li>Select your workspace</li>
        </ol>
        
        <a 
          href="https://api.slack.com/apps" 
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          Open Slack API Portal
        </a>
      </div>
      
      <UiButton @click="step = 2" class="w-full">
        I've created my Slack App
        <ArrowRight class="w-4 h-4 ml-2" />
      </UiButton>
    </div>
    
    <!-- Step 2: Configure OAuth & Permissions -->
    <div v-if="step === 2" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. Configure OAuth Scopes</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          In your Slack app settings, go to <strong>OAuth & Permissions</strong>:
        </p>
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Scroll to <strong>"Scopes"</strong> → <strong>"Bot Token Scopes"</strong></li>
          <li>Add these scopes:</li>
        </ol>
        <div class="flex flex-wrap gap-1 mt-2">
          <span v-for="scope in requiredScopes" :key="scope" class="text-xs bg-nanna-bg-deep px-2 py-1 rounded font-mono">
            {{ scope }}
          </span>
        </div>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 1" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 3" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 3: Install to Workspace & Get Token -->
    <div v-if="step === 3" class="space-y-4">
      <h4 class="font-medium text-nanna-text">3. Install & Get Bot Token</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Go back to <strong>OAuth & Permissions</strong></li>
          <li>Click <strong>"Install to Workspace"</strong></li>
          <li>Review and allow the permissions</li>
          <li>Copy the <strong>Bot User OAuth Token</strong> (starts with xoxb-)</li>
        </ol>
        
        <div class="bg-nanna-warning/10 border border-nanna-warning/30 rounded p-2 text-xs text-nanna-warning">
          ⚠️ Keep your token secret! Never share it publicly.
        </div>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Bot Token</label>
        <UiInput 
          v-model="botToken" 
          type="password"
          placeholder="xoxb-1234567890-..."
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Bot User OAuth Token (starts with xoxb-)
        </p>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">App Token <span class="text-nanna-text-dim">(for Socket Mode)</span></label>
        <UiInput 
          v-model="appToken" 
          type="password"
          placeholder="xapp-1-..."
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Generate at App-Level Tokens (starts with xapp-)
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 2" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 4" :disabled="!botToken" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 4: Enable Events & Test -->
    <div v-if="step === 4" class="space-y-4">
      <h4 class="font-medium text-nanna-text">4. Enable Events & Test</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          Enable Socket Mode for real-time events:
        </p>
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Go to <strong>Socket Mode</strong> and enable it</li>
          <li>Go to <strong>Event Subscriptions</strong> → Enable Events</li>
          <li>Subscribe to bot events: <code class="text-nanna-accent">message.channels</code>, <code class="text-nanna-accent">message.im</code>, <code class="text-nanna-accent">app_mention</code></li>
        </ol>
      </div>
      
      <!-- Optional: Channel Filter -->
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">
          Allowed Channels <span class="text-nanna-text-dim">(optional)</span>
        </label>
        <UiInput 
          v-model="allowedChannels" 
          placeholder="#general, #bot-test"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Comma-separated. Leave empty to allow all channels the bot is in.
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 3" variant="secondary">
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
    <div v-if="status?.configured" class="mt-4 pt-4 border-t border-nanna-primary/10">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <span class="w-2 h-2 rounded-full bg-nanna-success"></span>
          <span class="text-sm text-nanna-text">Currently connected</span>
        </div>
        <UiButton @click="$emit('test', 'Slack')" variant="ghost" size="sm">
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

const step = ref(props.status?.configured ? 4 : 1)
const botToken = ref('')
const appToken = ref('')
const allowedChannels = ref('')
const testing = ref(false)
const saving = ref(false)
const testResult = ref<{ success: boolean; message: string } | null>(null)

const requiredScopes = [
  'channels:history',
  'channels:read',
  'chat:write',
  'im:history',
  'im:read',
  'im:write',
  'reactions:read',
  'reactions:write',
  'users:read',
  'app_mentions:read',
]

async function testBot() {
  testing.value = true
  testResult.value = null
  
  try {
    if (!botToken.value.startsWith('xoxb-')) {
      testResult.value = { success: false, message: 'Invalid token format. Bot tokens start with xoxb-' }
      return
    }
    
    const response = await fetch('https://slack.com/api/auth.test', {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${botToken.value}`,
        'Content-Type': 'application/x-www-form-urlencoded',
      },
    })
    const data = await response.json()
    
    if (data.ok) {
      testResult.value = { 
        success: true, 
        message: `✓ Connected to ${data.team} as @${data.user}` 
      }
    } else {
      testResult.value = { success: false, message: `Slack Error: ${data.error}` }
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
  
  if (appToken.value) {
    config.app_token = appToken.value
  }
  
  if (allowedChannels.value) {
    config.allowed_channels = allowedChannels.value
  }
  
  emit('save', 'slack', config)
  saving.value = false
}
</script>
