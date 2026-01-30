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
    
    <!-- Step 1: Create Application -->
    <div v-if="step === 1" class="space-y-4">
      <h4 class="font-medium text-nanna-text">1. Create a Discord Application</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Go to the Discord Developer Portal</li>
          <li>Click <strong>"New Application"</strong></li>
          <li>Give it a name (e.g., "Nanna Bot")</li>
          <li>Copy the <strong>Application ID</strong> from General Information</li>
        </ol>
        
        <a 
          href="https://discord.com/developers/applications" 
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          Open Discord Developer Portal
        </a>
      </div>
      
      <UiButton @click="step = 2" class="w-full">
        I have my Application ID
        <ArrowRight class="w-4 h-4 ml-2" />
      </UiButton>
    </div>
    
    <!-- Step 2: Enter Application ID & Public Key -->
    <div v-if="step === 2" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. Application Details</h4>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Application ID</label>
        <UiInput 
          v-model="applicationId" 
          placeholder="123456789012345678"
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Found in General Information → Application ID
        </p>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Public Key</label>
        <UiInput 
          v-model="publicKey" 
          placeholder="abc123def456..."
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Found in General Information → Public Key
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 1" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 3" :disabled="!applicationId || !publicKey" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 3: Create Bot & Get Token -->
    <div v-if="step === 3" class="space-y-4">
      <h4 class="font-medium text-nanna-text">3. Create Bot & Get Token</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>In your application, go to <strong>"Bot"</strong> section</li>
          <li>Click <strong>"Add Bot"</strong> if you haven't already</li>
          <li>Click <strong>"Reset Token"</strong> to generate a new token</li>
          <li>Copy the token (you can only see it once!)</li>
        </ol>
        
        <div class="bg-nanna-warning/10 border border-nanna-warning/30 rounded p-2 text-xs text-nanna-warning">
          ⚠️ Keep your bot token secret! Never share it publicly.
        </div>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Bot Token</label>
        <UiInput 
          v-model="botToken" 
          type="password"
          placeholder="MTIzNDU2Nzg5.Gh12Ab.xxxxx..."
          class="font-mono"
        />
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
    
    <!-- Step 4: Invite & Test -->
    <div v-if="step === 4" class="space-y-4">
      <h4 class="font-medium text-nanna-text">4. Invite Bot & Test</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          Invite your bot to a server:
        </p>
        
        <a 
          :href="inviteUrl"
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          Invite Bot to Server
        </a>
        
        <p class="text-xs text-nanna-text-dim">
          The bot needs: Read Messages, Send Messages, Add Reactions
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
        <UiButton @click="saveConfig" :disabled="!isValid || saving" class="flex-1">
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
        <UiButton @click="$emit('test', 'Discord')" variant="ghost" size="sm">
          Test Connection
        </UiButton>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
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
const applicationId = ref('')
const publicKey = ref('')
const botToken = ref('')
const testing = ref(false)
const saving = ref(false)
const testResult = ref<{ success: boolean; message: string } | null>(null)

const isValid = computed(() => applicationId.value && publicKey.value && botToken.value)

const inviteUrl = computed(() => {
  if (!applicationId.value) return '#'
  // Permissions: Read Messages (1024) + Send Messages (2048) + Add Reactions (64) = 3136
  return `https://discord.com/api/oauth2/authorize?client_id=${applicationId.value}&permissions=3136&scope=bot`
})

async function testBot() {
  testing.value = true
  testResult.value = null
  
  try {
    // Try to call Discord API
    const response = await fetch('https://discord.com/api/v10/users/@me', {
      headers: {
        Authorization: `Bot ${botToken.value}`,
      },
    })
    
    if (response.ok) {
      const data = await response.json()
      testResult.value = { 
        success: true, 
        message: `✓ Connected as ${data.username}#${data.discriminator}` 
      }
    } else {
      const error = await response.json()
      testResult.value = { success: false, message: `API Error: ${error.message || response.statusText}` }
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
    application_id: applicationId.value,
    public_key: publicKey.value,
  }
  
  emit('save', 'discord', config)
  saving.value = false
}
</script>
