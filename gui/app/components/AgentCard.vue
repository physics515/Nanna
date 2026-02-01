<template>
  <div 
    :class="[
      'relative p-3 rounded-lg border cursor-pointer transition-all hover:shadow-md',
      getCardClasses(),
      depth > 0 && 'ml-4'
    ]"
    :style="{ marginLeft: `${depth * 16}px` }"
    @click="$emit('click')"
  >
    <!-- Connection line for sub-agents -->
    <div 
      v-if="depth > 0"
      class="absolute -left-4 top-1/2 w-4 h-px bg-nanna-primary/20"
    />

    <div class="flex items-center gap-3">
      <!-- State indicator -->
      <div :class="['relative w-10 h-10 rounded-lg flex items-center justify-center shrink-0', getStateBackground()]">
        <component :is="getStateIcon()" :class="['w-5 h-5', getStateTextColor()]" />
        
        <!-- Pulse animation for active states -->
        <div 
          v-if="isActiveState"
          :class="['absolute inset-0 rounded-lg animate-ping opacity-30', getStateBackground()]"
        />
      </div>

      <!-- Agent info -->
      <div class="flex-1 min-w-0">
        <div class="flex items-center gap-2 mb-0.5">
          <!-- Role badge -->
          <span :class="['px-1.5 py-0.5 rounded text-xs font-medium', getRoleBadge()]">
            {{ agent.role }}
          </span>
          
          <!-- State badge -->
          <span :class="['px-1.5 py-0.5 rounded text-xs', getStateBadge()]">
            {{ agent.state }}
          </span>
          
          <!-- Tool indicator -->
          <span v-if="agent.current_tool" class="flex items-center gap-1 text-xs text-nanna-accent">
            <Wrench class="w-3 h-3" />
            {{ agent.current_tool }}
          </span>
        </div>
        
        <div class="flex items-center gap-2">
          <code class="text-xs text-nanna-text-dim truncate">{{ agent.id }}</code>
          <span v-if="agent.children.length > 0" class="text-xs text-nanna-text-muted">
            · {{ agent.children.length }} sub-agent{{ agent.children.length !== 1 ? 's' : '' }}
          </span>
        </div>
      </div>

      <!-- Right side info -->
      <div class="flex items-center gap-3 shrink-0">
        <!-- Token usage -->
        <div class="hidden sm:block text-right">
          <div class="text-xs font-mono text-nanna-text-muted">
            {{ formatTokens(agent.tokens_in) }} / {{ formatTokens(agent.tokens_out) }}
          </div>
          <div class="text-xs text-nanna-text-dim">tokens</div>
        </div>

        <!-- Model -->
        <div class="hidden md:block px-2 py-1 rounded bg-nanna-bg-elevated text-xs text-nanna-text-muted">
          {{ formatModel(agent.model) }}
        </div>

        <!-- Cancel button -->
        <button
          v-if="!isTerminalState"
          @click.stop="$emit('cancel')"
          class="p-1.5 rounded-lg hover:bg-nanna-error/20 text-nanna-text-dim hover:text-nanna-error transition-colors"
          title="Cancel agent"
        >
          <X class="w-4 h-4" />
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { 
  Bot, Loader2, Wrench, CheckCircle, XCircle, Clock, 
  Pause, Sparkles, X
} from 'lucide-vue-next'

interface AgentInfo {
  id: string
  parent_id: string | null
  workspace_path: string | null
  workspace_name: string | null
  model: string
  role: string
  state: string
  state_changed_at: number
  spawned_at: number
  children: string[]
  current_tool: string | null
  tokens_in: number
  tokens_out: number
}

const props = defineProps<{
  agent: AgentInfo
  depth?: number
}>()

defineEmits<{
  (e: 'click'): void
  (e: 'cancel'): void
}>()

const depth = computed(() => props.depth || 0)

const isActiveState = computed(() => 
  ['thinking', 'tool_use', 'waiting'].includes(props.agent.state)
)

const isTerminalState = computed(() =>
  ['completed', 'error', 'cancelled'].includes(props.agent.state)
)

function getCardClasses(): string {
  const state = props.agent.state
  const base = 'bg-nanna-bg-surface'
  
  const stateClasses: Record<string, string> = {
    spawned: 'border-nanna-primary/10',
    idle: 'border-nanna-primary/20',
    thinking: 'border-nanna-warning/40 bg-nanna-warning/5',
    tool_use: 'border-nanna-accent/40 bg-nanna-accent/5',
    waiting: 'border-nanna-primary/30',
    completed: 'border-nanna-success/30 opacity-75',
    error: 'border-nanna-error/30 opacity-75',
    cancelled: 'border-nanna-text-dim/20 opacity-50',
  }
  
  return `${base} ${stateClasses[state] || 'border-nanna-primary/10'}`
}

function getStateBackground(): string {
  const map: Record<string, string> = {
    spawned: 'bg-nanna-bg-elevated',
    idle: 'bg-nanna-primary/20',
    thinking: 'bg-nanna-warning/20',
    tool_use: 'bg-nanna-accent/20',
    waiting: 'bg-nanna-primary/20',
    completed: 'bg-nanna-success/20',
    error: 'bg-nanna-error/20',
    cancelled: 'bg-nanna-text-dim/20',
  }
  return map[props.agent.state] || 'bg-nanna-bg-elevated'
}

function getStateTextColor(): string {
  const map: Record<string, string> = {
    spawned: 'text-nanna-text-dim',
    idle: 'text-nanna-primary',
    thinking: 'text-nanna-warning',
    tool_use: 'text-nanna-accent',
    waiting: 'text-nanna-primary',
    completed: 'text-nanna-success',
    error: 'text-nanna-error',
    cancelled: 'text-nanna-text-dim',
  }
  return map[props.agent.state] || 'text-nanna-text'
}

function getStateBadge(): string {
  const map: Record<string, string> = {
    spawned: 'bg-nanna-bg-elevated text-nanna-text-muted',
    idle: 'bg-nanna-primary/10 text-nanna-primary',
    thinking: 'bg-nanna-warning/10 text-nanna-warning',
    tool_use: 'bg-nanna-accent/10 text-nanna-accent',
    waiting: 'bg-nanna-primary/10 text-nanna-primary',
    completed: 'bg-nanna-success/10 text-nanna-success',
    error: 'bg-nanna-error/10 text-nanna-error',
    cancelled: 'bg-nanna-text-dim/10 text-nanna-text-dim',
  }
  return map[props.agent.state] || 'bg-nanna-bg-elevated text-nanna-text'
}

function getRoleBadge(): string {
  const map: Record<string, string> = {
    main: 'bg-nanna-accent/20 text-nanna-accent',
    subagent: 'bg-nanna-primary/20 text-nanna-primary',
    background: 'bg-nanna-text-dim/20 text-nanna-text-muted',
    system: 'bg-nanna-warning/20 text-nanna-warning',
  }
  return map[props.agent.role] || 'bg-nanna-bg-elevated text-nanna-text-muted'
}

function getStateIcon() {
  const map: Record<string, any> = {
    spawned: Bot,
    idle: Pause,
    thinking: Sparkles,
    tool_use: Wrench,
    waiting: Clock,
    completed: CheckCircle,
    error: XCircle,
    cancelled: X,
  }
  return map[props.agent.state] || Bot
}

function formatTokens(tokens: number): string {
  if (tokens >= 1000000) return `${(tokens / 1000000).toFixed(1)}M`
  if (tokens >= 1000) return `${(tokens / 1000).toFixed(1)}K`
  return tokens.toString()
}

function formatModel(model: string): string {
  return model
    .replace('claude-', '')
    .replace('-20250514', '')
    .replace('-20241022', '')
    .replace('gpt-', '')
}
</script>
