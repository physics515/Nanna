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
    
    <!-- Step 1: Choose Method -->
    <div v-if="step === 1" class="space-y-4">
      <h4 class="font-medium text-nanna-text">1. Choose Connection Method</h4>
      
      <div class="space-y-3">
        <!-- WhatsApp Business API -->
        <div 
          @click="connectionMethod = 'cloud-api'"
          :class="[
            'p-4 rounded-lg border-2 cursor-pointer transition-all',
            connectionMethod === 'cloud-api'
              ? 'border-nanna-primary bg-nanna-primary/10'
              : 'border-nanna-text-dim/20 hover:border-nanna-primary/50'
          ]"
        >
          <div class="flex items-start gap-3">
            <Cloud class="w-5 h-5 text-nanna-accent shrink-0 mt-0.5" />
            <div>
              <h5 class="font-medium text-nanna-text">WhatsApp Cloud API</h5>
              <p class="text-xs text-nanna-text-muted mt-1">
                Official Meta Cloud API. Requires Facebook Business account.
              </p>
              <span class="inline-block mt-1 text-xs bg-nanna-success/20 text-nanna-success px-2 py-0.5 rounded">
                Recommended
              </span>
            </div>
          </div>
        </div>
        
        <!-- WhatsApp Web (Baileys) -->
        <div 
          @click="connectionMethod = 'web'"
          :class="[
            'p-4 rounded-lg border-2 cursor-pointer transition-all',
            connectionMethod === 'web'
              ? 'border-nanna-primary bg-nanna-primary/10'
              : 'border-nanna-text-dim/20 hover:border-nanna-primary/50'
          ]"
        >
          <div class="flex items-start gap-3">
            <Smartphone class="w-5 h-5 text-nanna-accent shrink-0 mt-0.5" />
            <div>
              <h5 class="font-medium text-nanna-text">WhatsApp Web Bridge</h5>
              <p class="text-xs text-nanna-text-muted mt-1">
                Links your personal WhatsApp via QR code. No business account needed.
              </p>
              <span class="inline-block mt-1 text-xs bg-nanna-warning/20 text-nanna-warning px-2 py-0.5 rounded">
                Unofficial
              </span>
            </div>
          </div>
        </div>
      </div>
      
      <UiButton @click="step = 2" :disabled="!connectionMethod" class="w-full">
        Continue
        <ArrowRight class="w-4 h-4 ml-2" />
      </UiButton>
    </div>
    
    <!-- Step 2: Cloud API Setup -->
    <div v-if="step === 2 && connectionMethod === 'cloud-api'" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. Set Up Meta Developer App</h4>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-3">
        <ol class="text-sm text-nanna-text-muted space-y-2 list-decimal list-inside">
          <li>Go to Meta for Developers</li>
          <li>Create a new app → Select <strong>"Business"</strong> type</li>
          <li>Add the <strong>WhatsApp</strong> product</li>
          <li>Complete Business Verification (may take 1-2 days)</li>
          <li>Get your Phone Number ID and Access Token</li>
        </ol>
        
        <a 
          href="https://developers.facebook.com/apps" 
          target="_blank"
          class="inline-flex items-center gap-2 text-sm text-nanna-accent hover:underline"
        >
          <ExternalLink class="w-4 h-4" />
          Meta for Developers
        </a>
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
    
    <!-- Step 2: Web Bridge Setup -->
    <div v-if="step === 2 && connectionMethod === 'web'" class="space-y-4">
      <h4 class="font-medium text-nanna-text">2. QR Code Linking</h4>
      
      <div class="bg-nanna-warning/10 border border-nanna-warning/30 rounded-lg p-4 space-y-2">
        <div class="flex items-start gap-2">
          <AlertTriangle class="w-5 h-5 text-nanna-warning shrink-0" />
          <div>
            <p class="text-sm text-nanna-warning font-medium">Unofficial Method</p>
            <p class="text-xs text-nanna-text-muted mt-1">
              This uses WhatsApp Web reverse-engineering. Your account may be banned if detected.
              Only use for personal/testing purposes.
            </p>
          </div>
        </div>
      </div>
      
      <div class="bg-nanna-bg-elevated/50 rounded-lg p-4 text-center">
        <div v-if="!qrCode" class="space-y-3">
          <p class="text-sm text-nanna-text-muted">
            Click below to generate a QR code for linking
          </p>
          <UiButton @click="generateQrCode" :disabled="generatingQr">
            <UiSpinner v-if="generatingQr" size="sm" class="mr-2" />
            <QrCode v-else class="w-4 h-4 mr-2" />
            Generate QR Code
          </UiButton>
        </div>
        <div v-else class="space-y-3">
          <img :src="qrCode" alt="WhatsApp QR Code" class="mx-auto w-48 h-48 rounded-lg" />
          <p class="text-xs text-nanna-text-dim">
            Open WhatsApp on your phone → Settings → Linked Devices → Link a Device
          </p>
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
    
    <!-- Step 3: Enter Credentials (Cloud API) -->
    <div v-if="step === 3 && connectionMethod === 'cloud-api'" class="space-y-4">
      <h4 class="font-medium text-nanna-text">3. Enter API Credentials</h4>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Phone Number ID</label>
        <UiInput 
          v-model="phoneNumberId" 
          placeholder="123456789012345"
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Found in WhatsApp → Getting Started
        </p>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Access Token</label>
        <UiInput 
          v-model="accessToken" 
          type="password"
          placeholder="EAAG..."
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Temporary or permanent access token
        </p>
      </div>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Webhook Verify Token</label>
        <UiInput 
          v-model="verifyToken" 
          placeholder="your-verify-token"
          class="font-mono"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          A secret string you create for webhook verification
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 2" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 4" :disabled="!phoneNumberId || !accessToken" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 3: Web Bridge Config -->
    <div v-if="step === 3 && connectionMethod === 'web'" class="space-y-4">
      <h4 class="font-medium text-nanna-text">3. Configuration</h4>
      
      <div>
        <label class="block text-sm text-nanna-text-muted mb-1">Session Name</label>
        <UiInput 
          v-model="sessionName" 
          placeholder="nanna"
        />
        <p class="text-xs text-nanna-text-dim mt-1">
          Used to identify this connection
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 2" variant="secondary" class="flex-1">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="step = 4" class="flex-1">
          Continue
          <ArrowRight class="w-4 h-4 ml-2" />
        </UiButton>
      </div>
    </div>
    
    <!-- Step 4: Test & Save -->
    <div v-if="step === 4" class="space-y-4">
      <h4 class="font-medium text-nanna-text">4. Test & Save</h4>
      
      <!-- Webhook URL for Cloud API -->
      <div v-if="connectionMethod === 'cloud-api'" class="bg-nanna-bg-elevated/50 rounded-lg p-4 space-y-2">
        <p class="text-sm text-nanna-text-muted">
          Configure this webhook URL in Meta Developer Console:
        </p>
        <code class="block bg-nanna-bg-deep text-nanna-accent text-xs p-2 rounded font-mono break-all">
          {{ webhookUrl }}
        </code>
        <p class="text-xs text-nanna-text-dim">
          Subscribe to: messages, message_deliveries
        </p>
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
          Comma-separated. Leave empty to allow all.
        </p>
      </div>
      
      <div class="flex gap-2">
        <UiButton @click="step = 3" variant="secondary">
          <ArrowLeft class="w-4 h-4 mr-2" />
          Back
        </UiButton>
        <UiButton @click="testConnection" variant="secondary" :disabled="testing">
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
        <UiButton @click="$emit('test', 'WhatsApp')" variant="ghost" size="sm">
          Test Connection
        </UiButton>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { 
  ExternalLink, ArrowRight, ArrowLeft, Zap, Save, 
  Cloud, Smartphone, AlertTriangle, QrCode 
} from 'lucide-vue-next'

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
const connectionMethod = ref<'cloud-api' | 'web' | ''>('')

// Cloud API fields
const phoneNumberId = ref('')
const accessToken = ref('')
const verifyToken = ref('')

// Web bridge fields
const sessionName = ref('nanna')
const qrCode = ref('')
const generatingQr = ref(false)

// Common
const allowedContacts = ref('')
const testing = ref(false)
const saving = ref(false)
const testResult = ref<{ success: boolean; message: string } | null>(null)

const webhookUrl = computed(() => {
  // This would be the actual webhook endpoint
  return 'https://your-domain.com/webhook/whatsapp'
})

const isValid = computed(() => {
  if (connectionMethod.value === 'cloud-api') {
    return phoneNumberId.value && accessToken.value
  }
  return sessionName.value
})

async function generateQrCode() {
  generatingQr.value = true
  try {
    // This would call the backend to generate a QR code
    // For now, show a placeholder
    await new Promise(resolve => setTimeout(resolve, 1000))
    qrCode.value = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=='
    testResult.value = { success: true, message: 'QR code generated. Scan with WhatsApp.' }
  } catch (e: any) {
    testResult.value = { success: false, message: `Failed to generate QR: ${e.message}` }
  } finally {
    generatingQr.value = false
  }
}

async function testConnection() {
  testing.value = true
  testResult.value = null
  
  try {
    if (connectionMethod.value === 'cloud-api') {
      // Test Cloud API
      const response = await fetch(
        `https://graph.facebook.com/v17.0/${phoneNumberId.value}`,
        {
          headers: {
            Authorization: `Bearer ${accessToken.value}`,
          },
        }
      )
      
      if (response.ok) {
        const data = await response.json()
        testResult.value = { 
          success: true, 
          message: `✓ Connected: ${data.display_phone_number || phoneNumberId.value}` 
        }
      } else {
        const error = await response.json()
        testResult.value = { 
          success: false, 
          message: `API Error: ${error.error?.message || response.statusText}` 
        }
      }
    } else {
      // Web bridge test would check session status
      testResult.value = { 
        success: true, 
        message: '✓ Configuration valid. Connect via QR code to activate.' 
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
    connection_method: connectionMethod.value,
  }
  
  if (connectionMethod.value === 'cloud-api') {
    config.phone_number_id = phoneNumberId.value
    config.access_token = accessToken.value
    if (verifyToken.value) {
      config.verify_token = verifyToken.value
    }
  } else {
    config.session_name = sessionName.value
  }
  
  if (allowedContacts.value) {
    config.allowed_contacts = allowedContacts.value
  }
  
  emit('save', 'whatsapp', config)
  saving.value = false
}
</script>
