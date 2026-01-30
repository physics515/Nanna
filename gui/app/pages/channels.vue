<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center justify-between gap-3">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Channels</h2>
          <p class="text-xs sm:text-sm text-nanna-text-muted">
            Messaging channel integrations
          </p>
        </div>
        <UiButton @click="refreshStatus" variant="secondary" size="sm" :disabled="isLoading">
          {{ isLoading ? '⏳ Loading...' : '🔄 Refresh' }}
        </UiButton>
      </div>
    </header>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <!-- Info banner -->
      <UiCard class="mb-4 sm:mb-6 bg-nanna-accent/10 border-nanna-accent/20">
        <div class="flex gap-3">
          <span class="text-xl sm:text-2xl">💡</span>
          <div>
            <h3 class="font-medium text-nanna-accent text-sm sm:text-base mb-1">About Channels</h3>
            <p class="text-xs sm:text-sm text-nanna-text-muted">
              Channels allow Nanna to communicate via messaging platforms like Telegram, Discord, and Slack.
              The desktop app shows chat locally, but you can configure channels for remote access via the server.
            </p>
          </div>
        </div>
      </UiCard>

      <!-- Channel grid -->
      <div class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-3 sm:gap-4">
        <UiCard
          v-for="channel in channels"
          :key="channel.name"
          :class="[
            'border-2 transition-colors',
            channel.configured 
              ? 'border-nanna-success/30 hover:border-nanna-success/50' 
              : 'border-nanna-text-dim/10 hover:border-nanna-text-dim/20'
          ]"
        >
          <!-- Header -->
          <div class="flex items-center justify-between mb-3">
            <div class="flex items-center gap-2 sm:gap-3">
              <span class="text-2xl sm:text-3xl">{{ getChannelIcon(channel.name) }}</span>
              <div>
                <h3 class="font-semibold text-nanna-text text-sm sm:text-base">{{ channel.name }}</h3>
                <UiBadge :variant="channel.status === 'ready' ? 'success' : 'secondary'" class="text-xs">
                  {{ channel.status === 'ready' ? '✓ Configured' : 'Not configured' }}
                </UiBadge>
              </div>
            </div>
          </div>

          <!-- Details -->
          <div v-if="channel.details" class="text-xs sm:text-sm text-nanna-text-muted mb-3">
            {{ channel.details }}
          </div>

          <!-- Status indicator -->
          <div class="flex items-center gap-2 text-xs sm:text-sm">
            <span :class="[
              'w-2 h-2 rounded-full shrink-0',
              channel.configured ? 'bg-nanna-success' : 'bg-nanna-text-dim'
            ]"></span>
            <span class="text-nanna-text-dim">
              {{ channel.configured ? 'Ready for server mode' : 'Configure in config.toml' }}
            </span>
          </div>
        </UiCard>
      </div>

      <!-- Configuration help -->
      <UiCard class="mt-6 sm:mt-8">
        <h3 class="font-semibold text-nanna-text text-sm sm:text-base mb-3">Configuration</h3>
        <p class="text-xs sm:text-sm text-nanna-text-muted mb-3">
          Channel credentials are stored in your config file:
        </p>
        <code class="block bg-nanna-bg-deep text-nanna-accent text-xs sm:text-sm p-3 rounded font-mono break-all">
          {{ configPath }}
        </code>
        <p class="text-xs sm:text-sm text-nanna-text-muted mt-3">
          Edit this file to add channel tokens and credentials. See the 
          <a href="https://docs.clawd.bot" class="text-nanna-accent hover:underline" target="_blank">documentation</a>
          for setup guides.
        </p>
      </UiCard>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface ChannelStatus {
  name: string
  configured: boolean
  enabled: boolean
  status: string
  details: string | null
}

const channels = ref<ChannelStatus[]>([])
const isLoading = ref(false)

const configPath = computed(() => {
  // Approximate path - actual path depends on OS
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
  try {
    channels.value = await invoke<ChannelStatus[]>('get_channel_status')
  } catch (e) {
    console.error('Failed to load channel status:', e)
  } finally {
    isLoading.value = false
  }
}

function getChannelIcon(name: string): string {
  const icons: Record<string, string> = {
    'Telegram': '✈️',
    'Discord': '🎮',
    'Slack': '💼',
    'Signal': '🔒',
    'WhatsApp': '💬',
  }
  return icons[name] || '📱'
}
</script>
