<template>
  <section v-if="servers.length > 0" class="text-left" aria-label="MCP servers">
    <h4 class="text-xs font-semibold text-nanna-text-muted uppercase tracking-wide mb-2">MCP servers</h4>
    <ul class="space-y-1">
      <li
        v-for="(server, index) in servers"
        :key="`${server.name}-${index}`"
        class="flex items-start gap-2 text-xs"
        :data-state="server.state"
      >
        <span
          class="mt-1 inline-block w-2 h-2 rounded-full flex-shrink-0"
          :class="{
            'bg-emerald-400': server.state === 'started',
            'bg-amber-400': server.state === 'starting',
            'bg-red-400': server.state === 'failed',
            'bg-nanna-text-dim': server.state === 'not_started',
          }"
        />
        <span class="font-mono text-nanna-text">{{ server.name || '(unnamed)' }}</span>
        <span class="text-nanna-text-muted break-words">{{ describeMcpServer(server) }}</span>
      </li>
    </ul>
  </section>
</template>

<script setup lang="ts">
import { describeMcpServer, type McpServerState } from '~/lib/mcpServers'

defineProps<{ servers: McpServerState[] }>()
</script>
