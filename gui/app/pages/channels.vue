<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04] bg-nanna-bg-surface/80">
      <div class="flex items-center justify-between gap-3">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Channels</h2>
          <p class="text-xs sm:text-sm text-nanna-text-muted">
            Connect Nanna to messaging platforms
          </p>
        </div>
        <UiButton @click="refreshStatus" variant="secondary" size="sm" :disabled="isLoading">
          <RefreshCw :class="['w-4 h-4', isLoading && 'animate-spin']" />
        </UiButton>
      </div>
    </header>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <div class="max-w-3xl mx-auto space-y-4">
        
        <PageState
          v-if="isLoading || !isOnline || loadError"
          :state="isLoading ? 'loading' : (!isOnline ? 'offline' : 'error')"
          :title="isLoading ? 'Loading channels…' : (!isOnline ? 'Daemon offline' : 'Could not load channels')"
          :description="isLoading
            ? 'Reading channel onboarding state from the daemon.'
            : (!isOnline
              ? 'Channel listeners run inside the daemon. Reconnect to configure Telegram, Discord, Slack, Signal, or WhatsApp.'
              : (loadError || 'Unknown error'))"
          :primary-action="isLoading ? '' : 'Retry'"
          :primary-busy="isLoading"
          @primary="refreshStatus"
        />

        <!-- Channel Cards -->
        <UiCard
          v-for="channel in channelConfigs"
          :key="channel.id"
          :class="'border-2 transition-all ' + (getChannelStatus(channel.id)?.configured 
              ? 'border-nanna-success/30' 
              : 'border-nanna-text-dim/10')"
        >
          <!-- Channel Header -->
          <div 
            class="flex items-center justify-between cursor-pointer"
            @click="toggleChannel(channel.id)"
          >
            <div class="flex items-center gap-3">
              <span class="text-2xl">{{ channel.icon }}</span>
              <div>
                <h3 class="font-semibold text-nanna-text">{{ channel.name }}</h3>
                <p class="text-xs text-nanna-text-dim">{{ channel.description }}</p>
              </div>
            </div>
            <div class="flex items-center gap-2">
              <UiBadge 
                v-if="getChannelStatus(channel.id)?.configured" 
                variant="success"
              >
                ✓ Connected
              </UiBadge>
              <UiBadge v-else variant="secondary">Not configured</UiBadge>
              <ChevronDown 
                :class="[
                  'w-5 h-5 text-nanna-text-muted transition-transform',
                  expandedChannel === channel.id && 'rotate-180'
                ]" 
              />
            </div>
          </div>
          
          <!-- Expanded Setup Wizard -->
          <Transition name="expand">
            <div v-if="expandedChannel === channel.id" class="mt-4 pt-4 border-t border-white/[0.04]">
              
              <!-- Telegram Setup -->
              <TelegramSetup 
                v-if="channel.id === 'telegram'"
                :status="getChannelStatus('telegram')"
                @save="saveChannelConfig"
                @test="testConnection"
              />
              
              <!-- Discord Setup -->
              <DiscordSetup 
                v-if="channel.id === 'discord'"
                :status="getChannelStatus('discord')"
                @save="saveChannelConfig"
                @test="testConnection"
              />
              
              <!-- Slack Setup -->
              <SlackSetup 
                v-if="channel.id === 'slack'"
                :status="getChannelStatus('slack')"
                @save="saveChannelConfig"
                @test="testConnection"
              />
              
              <!-- Signal Setup -->
              <SignalSetup 
                v-if="channel.id === 'signal'"
                :status="getChannelStatus('signal')"
                @save="saveChannelConfig"
                @test="testConnection"
              />
              
              <!-- WhatsApp Setup -->
              <WhatsAppSetup 
                v-if="channel.id === 'whatsapp'"
                :status="getChannelStatus('whatsapp')"
                @save="saveChannelConfig"
                @test="testConnection"
              />
              
            </div>
          </Transition>
        </UiCard>
        
        <!-- Live Status Dashboard -->
        <UiCard>
          <div class="flex items-center justify-between mb-4">
            <h3 class="font-semibold text-nanna-primary flex items-center gap-2">
              <Activity class="w-4 h-4" />
              Live Status
            </h3>
          </div>
          <ChannelStatusLive />
        </UiCard>
        
        <!-- Config File Info -->
        <UiCard class="bg-nanna-bg-elevated/30">
          <div class="flex items-start gap-3">
            <FileCode class="w-5 h-5 text-nanna-accent shrink-0 mt-0.5" />
            <div>
              <h3 class="font-medium text-nanna-text text-sm mb-1">Configuration File</h3>
              <p class="text-xs text-nanna-text-dim mb-2">
                Channel credentials are saved to your config file:
              </p>
              <code class="block bg-nanna-bg-deep text-nanna-accent text-xs p-2 rounded font-mono break-all">
                {{ configPath }}
              </code>
            </div>
          </div>
        </UiCard>
        
      </div>
    </div>
    
    <!-- Toast -->
    <Transition name="toast">
      <div 
        v-if="toast" 
        :class="[
          'fixed bottom-4 right-4 left-4 sm:left-auto px-4 py-3 rounded-lg shadow-lg flex items-center gap-2 max-w-sm mx-auto sm:mx-0 z-50',
          toast.type === 'success' ? 'bg-nanna-success text-nanna-bg-deep' : 'bg-nanna-error text-white'
        ]"
      >
        <CheckCircle v-if="toast.type === 'success'" class="w-4 h-4 shrink-0" />
        <XCircle v-else class="w-4 h-4 shrink-0" />
        <span class="text-sm">{{ toast.message }}</span>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { RefreshCw, ChevronDown, FileCode, CheckCircle, XCircle, Activity } from 'lucide-vue-next'

const { isOnline } = useBackend()

interface ChannelStatus {
  name: string
  configured: boolean
  enabled: boolean
  status: string
  details: string | null
}

interface ChannelConfig {
  id: string
  name: string
  icon: string
  description: string
}

const channelConfigs: ChannelConfig[] = [
  { id: 'telegram', name: 'Telegram', icon: '✈️', description: 'Bot messaging via Telegram' },
  { id: 'discord', name: 'Discord', icon: '🎮', description: 'Server and DM messaging' },
  { id: 'slack', name: 'Slack', icon: '💼', description: 'Workspace messaging' },
  { id: 'signal', name: 'Signal', icon: '🔒', description: 'Secure private messaging' },
  { id: 'whatsapp', name: 'WhatsApp', icon: '💬', description: 'WhatsApp Business API' },
]

const channels = ref<ChannelStatus[]>([])
const isLoading = ref(false)
const loadError = ref<string | null>(null)
const expandedChannel = ref<string | null>(null)
const toast = ref<{ message: string; type: 'success' | 'error' } | null>(null)

const configPath = computed(() => {
  if (navigator.platform.includes('Win')) {
    return '%APPDATA%\\clawd\\Nanna\\config\\config.toml'
  } else if (navigator.platform.includes('Mac')) {
    return '~/Library/Application Support/clawd.Nanna/config.toml'
  } else {
    return '~/.config/nanna/config.toml'
  }
})

onMounted(async () => {
  await refreshStatus()
})

async function refreshStatus() {
  isLoading.value = true
  loadError.value = null
  try {
    channels.value = await invoke<ChannelStatus[]>('get_channel_status')
  } catch (e) {
    console.error('Failed to load channel status:', e)
    loadError.value = e instanceof Error ? e.message : String(e)
  } finally {
    isLoading.value = false
  }
}

function getChannelStatus(id: string): ChannelStatus | undefined {
  const nameMap: Record<string, string> = {
    telegram: 'Telegram',
    discord: 'Discord',
    slack: 'Slack',
    signal: 'Signal',
    whatsapp: 'WhatsApp',
  }
  return channels.value.find(c => c.name === nameMap[id])
}

function toggleChannel(id: string) {
  expandedChannel.value = expandedChannel.value === id ? null : id
}

async function saveChannelConfig(channel: string, config: Record<string, string>) {
  try {
    await invoke('save_channel_config', { channel, config })
    showToast(`${channel} configuration saved`, 'success')
    await refreshStatus()
  } catch (e: any) {
    showToast(`Failed to save: ${e.message || e}`, 'error')
  }
}

async function testConnection(channel: string) {
  try {
    const result = await invoke<{ success: boolean; message: string }>('test_channel_connection', { channel })
    if (result.success) {
      showToast(`${channel}: ${result.message}`, 'success')
    } else {
      showToast(`${channel}: ${result.message}`, 'error')
    }
  } catch (e: any) {
    showToast(`Test failed: ${e.message || e}`, 'error')
  }
}

function showToast(message: string, type: 'success' | 'error') {
  toast.value = { message, type }
  setTimeout(() => { toast.value = null }, 4000)
}
</script>

<style scoped>
.expand-enter-active,
.expand-leave-active {
  transition: all 0.3s ease;
  overflow: hidden;
}
.expand-enter-from,
.expand-leave-to {
  opacity: 0;
  max-height: 0;
}
.expand-enter-to,
.expand-leave-from {
  opacity: 1;
  max-height: 500px;
}

.toast-enter-active,
.toast-leave-active {
  transition: all 0.3s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(10px);
}
</style>
