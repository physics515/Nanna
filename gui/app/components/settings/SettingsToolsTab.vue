<template>
  <div class="space-y-6">
    <!-- Tool API Keys -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <Key class="w-4 h-4" />
        Tool API Keys
      </h3>
      <div class="space-y-4">
        <ApiKeyInput
          label="Brave Search"
          provider="brave"
          placeholder="BSA..."
          :is-set="settings?.brave_key_set"
          hint="For web_search tool"
          @save="saveApiKey"
        />
      </div>
    </UiCard>

    <!-- Available Tools -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <Wrench class="w-4 h-4" />
        Available Tools
        <UiBadge variant="outline" class="ml-auto">{{ settings?.tools?.length || 0 }}</UiBadge>
      </h3>
      <div class="space-y-2">
        <div
          v-for="tool in settings?.tools || []"
          :key="tool.name"
          class="flex items-center justify-between gap-2 p-3 rounded-lg bg-nanna-bg-elevated/40"
        >
          <div class="flex items-center gap-2 min-w-0">
            <span class="text-lg">{{ getToolIcon(tool.name) }}</span>
            <div class="min-w-0">
              <span class="text-sm font-medium text-nanna-text font-mono">{{ tool.name }}</span>
              <p class="text-xs text-nanna-text-dim truncate">{{ tool.description }}</p>
            </div>
          </div>
          <UiBadge :variant="tool.enabled ? 'success' : 'outline'" class="shrink-0">
            {{ tool.enabled ? 'Active' : 'Off' }}
          </UiBadge>
        </div>
      </div>
    </UiCard>
  </div>
</template>

<script setup lang="ts">
import { Key, Wrench } from 'lucide-vue-next'
import { useSettingsPage } from '~/composables/useSettingsPage'

const store = useSettingsPage()
const { settings, saveApiKey } = store

function getToolIcon(name: string): string {
  const icons: Record<string, string> = {
    read_file: '📄', write_file: '✏️', list_dir: '📁', exec: '⚡',
    web_fetch: '🌐', web_search: '🔍', echo: '💬', analyze_image: '👁️',
    ocr: '📝', describe_image: '🖼️', read_pdf: '📑',
  }
  return icons[name] || '🔧'
}
</script>
