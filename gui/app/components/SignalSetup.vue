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
    
    <!-- Step 1: Choose Method -->
    <div v-if="step === 1" class="space-y-4">
      <h4 class="font-medium text-nanna-text">1. Choose Connection Method</h4>
      
      <div class="space-y-3">
        <!-- signal-cli option -->
        <div 
          @click="connectionMethod = 'signal-cli'"
          :class="[
            'p-4 rounded-lg border-2 cursor-pointer transition-all',
            connectionMethod === 'signal-cli'
              ? 'border-nanna-primary bg-nanna-primary/10'
              : 'border-nanna-text-dim/20 hover:border-nanna-primary/50'
          ]"
        >
          <div class="flex items-start gap-3">
            <Terminal class="w-5 h-5 text-nanna-accent shrink-0 mt-0.5" />
            <div>
              <h5 class="font-medium text-nanna-text">signal-cli (Recommended)</h5>
              <p class="text-xs text-nanna-text-muted mt-1">
                Uses signal-cli for full Signal functionality. Requires Java runtime.
              </p>
            </div>
          </div>
        </div>
        
        <!-- Signal API option -->
        <div 
          @click="connectionMethod = 'api'"
          :class="[
            'p-4 rounded-lg border-2 cursor-pointer transition-all',
            connectionMethod === 'api'
              ? 'border-nanna-primary bg-nanna-primary/10'
              : 'border-nanna-text-dim/20 hover:border-nanna-primary/50'
          ]"
        >
          <div class="flex items-start gap-3">
            <Webhook class="w-5 h-5 text-nanna-accent shrink-0 mt-0.5" />
            <div>
              <h5 class="font-medium text-nanna-text">Signal REST API</h5>
              <p class="text-xs text-nanna-text-muted mt-1">
                Connect to a running signal-cli-rest-api instance (Docker).
              </p>
            </div>
          </div>
        </div>
      </div>
      
      <UiButton @click="step = 2" :disabled="!connectionMethod" class="w-full">
        Continue
        <ArrowRight class="w-4 h-4 ml-2" />
      </UiButton>
    </div>
    
    <!-- Step 2: Configure (signal-cli) -->
    <div v-if="step === 2 && connectionMethod === 'signal-cli'" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. Configure signal-cli</h4>
      
      <div class="bg-nanna-bg-elevated/40 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          First, install signal-cli:
        </p>
        <div class="bg-nanna-bg-deep p-2 rounded font-mono text-xs text-nanna-accent overflow-x-auto">
          # macOS<br>
          brew install signal-cli<br><br>
          # Linux (download from GitHub)<br>
          # Windows: Use WSL or download release
        </div>
        
        <a 
          href="https://github.com/AsamK/signal-cli" 
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          signal-cli GitHub
        </a>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Phone Number</label>
        <UiInput 
          v-model="phoneNumber" 
          placeholder="+1234567890"
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Your Signal phone number in international format
        </p>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">signal-cli Path <span class="text-nanna-text-dim">(optional)</span></label>
        <UiInput 
          v-model="signalCliPath" 
          placeholder="/usr/local/bin/signal-cli"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Leave empty to use default PATH
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 1" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 3" :disabled="!phoneNumber" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 2: Configure (REST API) -->
    <div v-if="step === 2 && connectionMethod === 'api'" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. Configure Signal REST API</h4>
      
      <div class="bg-nanna-bg-elevated/40 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          Run the signal-cli-rest-api Docker container:
        </p>
        <div class="bg-nanna-bg-deep p-2 rounded font-mono text-xs text-nanna-accent overflow-x-auto whitespace-pre">
docker run -d --name signal-api \
  -p 8080:8080 \
  -v signal-cli-config:/home/.local/share/signal-cli \
  bbernhard/signal-cli-rest-api</div>
        
        <a 
          href="https://github.com/bbernhard/signal-cli-rest-api" 
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          signal-cli-rest-api GitHub
        </a>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">API Base URL</label>
        <UiInput 
          v-model="apiUrl" 
          placeholder="http://localhost:8080"
        />
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Phone Number</label>
        <UiInput 
          v-model="phoneNumber" 
          placeholder="+1234567890"
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          The registered Signal number on the API instance
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 1" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 3" :disabled="!apiUrl || !phoneNumber" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 3: Link & Test -->
    <div v-if="step === 3" class="space-y-4">
      <h4 class="font-medium text-nanna-text">3. Link Account & Test</h4>
      
      <div v-if="!isLinked" class="bg-nanna-bg-elevated/40 rounded-lg p-4 space-y-3">
        <p class="text-sm text-nanna-text-muted">
          If this is a new setup, you need to link your Signal account:
        </p>
        
        <div v-if="connectionMethod === 'signal-cli'" class="bg-nanna-bg-deep p-2 rounded font-mono text-xs text-nanna-accent">
          signal-cli link -n "Nanna"
        </div>
        <div v-else class="text-sm text-nanna-text-dim">
          Use the /v1/qrcodelink endpoint to generate a QR code
        </div>
        
        <UiButton @click="markLinked" variant="secondary" size="sm">
          I've linked my account
        </UiButton>
      </div>
      
      <!-- Allowed Contacts -->
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">
          Allowed Contacts <span class="text-nanna-text-dim">(optional)</span>
        </label>
        <UiInput 
          v-model="allowedContacts" 
          placeholder="+1234567890, +0987654321"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Comma-separated phone numbers. Leave empty to allow all.
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 2" variant="secondary">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="testConnection" variant="secondary" :disabled="testing">
          <UiSpinner v-if="testing" size="sm" class="mr-2" />
          <Zap v-else class="w-4 h-4 mr-2" />
          Test
        </UiButton>
        <UiButton @click="saveConfig" :disabled="!phoneNumber || saving" class="flex-1">
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
        <UiButton @click="$emit('test', 'Signal')" variant="ghost" size="sm">
          Test Connection
        </UiButton>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { ExternalLink, ArrowRight, ArrowLeft, Zap, Save, Terminal, Webhook } from 'lucide-vue-next'

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
const connectionMethod = ref<'signal-cli' | 'api' | ''>('')
const phoneNumber = ref('')
const signalCliPath = ref('')
const apiUrl = ref('http://localhost:8080')
const allowedContacts = ref('')
const isLinked = ref(false)
const testing = ref(false)
const saving = ref(false)
const testResult = ref<{ success: boolean; message: string } | null>(null)

function markLinked() {
  isLinked.value = true
}

async function testConnection() {
  testing.value = true
  testResult.value = null
  
  try {
    if (connectionMethod.value === 'api') {
      // Test REST API connection
      const response = await fetch(`${apiUrl.value}/v1/about`)
      if (response.ok) {
        const data = await response.json()
        testResult.value = { 
          success: true, 
          message: `✓ Connected to Signal API v${data.versions?.['signal-cli'] || 'unknown'}` 
        }
      } else {
        testResult.value = { success: false, message: `API not reachable: ${response.statusText}` }
      }
    } else {
      // For signal-cli, we can't really test without invoking it
      testResult.value = { 
        success: true, 
        message: '✓ Configuration looks valid. Will test on first message.' 
      }
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
    phone_number: phoneNumber.value,
    connection_method: connectionMethod.value,
  }
  
  if (connectionMethod.value === 'signal-cli' && signalCliPath.value) {
    config.signal_cli_path = signalCliPath.value
  }
  
  if (connectionMethod.value === 'api') {
    config.api_url = apiUrl.value
  }
  
  if (allowedContacts.value) {
    config.allowed_contacts = allowedContacts.value
  }
  
  emit('save', 'signal', config)
  saving.value = false
}
</script>
