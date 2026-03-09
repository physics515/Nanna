<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04] bg-nanna-bg-surface/80">
      <div class="flex items-center justify-between gap-3">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-nanna-text flex items-center gap-2">
            <Bot class="w-5 h-5 text-nanna-accent" />
            Agent Overview
          </h2>
          <p class="text-xs sm:text-sm text-nanna-text-muted">
            Real-time view of all active agents across workspaces
          </p>
        </div>
        <div class="flex items-center gap-3">
          <!-- Stats badges -->
          <div class="hidden sm:flex items-center gap-2 text-sm">
            <span class="px-2 py-1 rounded-lg bg-nanna-bg-elevated text-nanna-text-muted">
              {{ stats?.total_agents || 0 }} agents
            </span>
            <span v-if="stats?.active_agents" class="px-2 py-1 rounded-lg bg-nanna-success/20 text-nanna-success">
              {{ stats.active_agents }} active
            </span>
          </div>
          <UiButton @click="refreshAgents" variant="ghost" size="sm" :disabled="isLoading">
            <RefreshCw :class="['w-4 h-4', isLoading && 'animate-spin']" />
          </UiButton>
        </div>
      </div>
    </header>

    <!-- Main content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <div class="max-w-6xl mx-auto space-y-6">

        <!-- Empty state -->
        <div v-if="!isLoading && clusters.length === 0" class="text-center py-16">
          <div class="w-20 h-20 mx-auto mb-4 rounded-full bg-nanna-bg-elevated flex items-center justify-center">
            <Bot class="w-10 h-10 text-nanna-text-dim" />
          </div>
          <h3 class="text-xl font-semibold text-nanna-text mb-2">No agents running</h3>
          <p class="text-sm text-nanna-text-muted max-w-md mx-auto">
            Start a chat session or run a task to see agents appear here.
            Each workspace can have its own main agent with sub-agents.
          </p>
        </div>

        <!-- Global stats bar -->
        <div v-if="stats && stats.total_agents > 0" class="grid grid-cols-2 sm:grid-cols-4 gap-3">
          <div class="p-3 rounded-xl bg-nanna-bg-elevated/30 border border-white/[0.04]">
            <div class="text-2xl font-bold text-nanna-text">{{ stats.total_agents }}</div>
            <div class="text-xs text-nanna-text-muted">Total Agents</div>
          </div>
          <div class="p-3 rounded-xl bg-nanna-success/10 border border-nanna-success/20">
            <div class="text-2xl font-bold text-nanna-success">{{ stats.active_agents }}</div>
            <div class="text-xs text-nanna-text-muted">Active</div>
          </div>
          <div class="p-3 rounded-xl bg-nanna-bg-elevated/30 border border-white/[0.04]">
            <div class="text-2xl font-bold text-nanna-text">{{ stats.workspaces }}</div>
            <div class="text-xs text-nanna-text-muted">Workspaces</div>
          </div>
          <div class="p-3 rounded-xl bg-nanna-bg-elevated/30 border border-white/[0.04]">
            <div class="text-lg font-mono text-nanna-text">
              {{ formatTokens(stats.total_tokens_in) }} / {{ formatTokens(stats.total_tokens_out) }}
            </div>
            <div class="text-xs text-nanna-text-muted">Tokens In/Out</div>
          </div>
        </div>

        <!-- Workspace clusters -->
        <div v-for="cluster in clusters" :key="cluster.path" class="space-y-3">
          <!-- Cluster header -->
          <div 
            class="flex items-center justify-between cursor-pointer group"
            @click="toggleCluster(cluster.path)"
          >
            <div class="flex items-center gap-3">
              <div class="w-8 h-8 rounded-lg bg-nanna-accent/20 flex items-center justify-center">
                <FolderOpen class="w-4 h-4 text-nanna-accent" />
              </div>
              <div>
                <h3 class="font-semibold text-nanna-text group-hover:text-nanna-accent transition-colors">
                  {{ cluster.name }}
                </h3>
                <div class="text-xs text-nanna-text-dim">{{ cluster.path }}</div>
              </div>
            </div>
            <div class="flex items-center gap-3">
              <span class="text-sm text-nanna-text-muted">
                {{ cluster.total_agents }} agent{{ cluster.total_agents !== 1 ? 's' : '' }}
              </span>
              <span v-if="cluster.active_agents > 0" class="text-sm text-nanna-success">
                {{ cluster.active_agents }} active
              </span>
              <ChevronDown 
                :class="[
                  'w-4 h-4 text-nanna-text-muted transition-transform',
                  !collapsedClusters[cluster.path] && 'rotate-180'
                ]"
              />
            </div>
          </div>

          <!-- Agent cards -->
          <div v-if="!collapsedClusters[cluster.path]" class="ml-4 pl-4 border-l border-white/[0.04] space-y-2">
            <AgentCard
              v-for="agent in cluster.agents"
              :key="agent.id"
              :agent="agent"
              :depth="getAgentDepth(agent, cluster.agents)"
              @click="selectAgent(agent)"
              @cancel="cancelAgent(agent.id)"
            />
          </div>
        </div>

        <!-- State legend -->
        <div v-if="stats && stats.total_agents > 0" class="mt-8 p-4 rounded-xl bg-nanna-bg-elevated/40 border border-white/[0.04]">
          <h4 class="text-xs font-semibold text-nanna-text-muted uppercase mb-3">State Legend</h4>
          <div class="flex flex-wrap gap-3">
            <div v-for="(color, state) in stateColors" :key="state" class="flex items-center gap-2">
              <div :class="['w-3 h-3 rounded-full', color]" />
              <span class="text-xs text-nanna-text-muted capitalize">{{ state }}</span>
            </div>
          </div>
        </div>

      </div>
    </div>

    <!-- Agent detail sidebar -->
    <Teleport to="body">
      <div 
        v-if="selectedAgent" 
        class="fixed inset-0 bg-black/60 flex justify-end z-50"
        @click.self="selectedAgent = null"
      >
        <div class="w-full max-w-md bg-nanna-bg-surface h-full overflow-y-auto border-l border-white/[0.06] shadow-2xl">
          <!-- Sidebar header -->
          <div class="sticky top-0 glass border-b border-white/[0.04] p-4 flex items-center justify-between">
            <div class="flex items-center gap-3">
              <div :class="['w-10 h-10 rounded-lg flex items-center justify-center', getStateBackground(selectedAgent.state)]">
                <Bot :class="['w-5 h-5', getStateTextColor(selectedAgent.state)]" />
              </div>
              <div>
                <h3 class="font-semibold text-nanna-text">Agent Details</h3>
                <code class="text-xs text-nanna-text-dim">{{ selectedAgent.id }}</code>
              </div>
            </div>
            <UiButton @click="selectedAgent = null" variant="ghost" size="sm">
              <X class="w-4 h-4" />
            </UiButton>
          </div>

          <!-- Sidebar content -->
          <div class="p-4 space-y-4">
            <!-- Status -->
            <div class="p-3 rounded-lg bg-nanna-bg-elevated">
              <div class="flex items-center justify-between mb-2">
                <span class="text-sm text-nanna-text-muted">Status</span>
                <span :class="['px-2 py-0.5 rounded text-xs font-medium', getStateBadge(selectedAgent.state)]">
                  {{ selectedAgent.state }}
                </span>
              </div>
              <div v-if="selectedAgent.current_tool" class="text-sm text-nanna-accent">
                Executing: {{ selectedAgent.current_tool }}
              </div>
            </div>

            <!-- Info grid -->
            <div class="grid grid-cols-2 gap-3">
              <div class="p-3 rounded-lg bg-nanna-bg-elevated">
                <div class="text-xs text-nanna-text-muted">Role</div>
                <div class="text-sm text-nanna-text capitalize">{{ selectedAgent.role }}</div>
              </div>
              <div class="p-3 rounded-lg bg-nanna-bg-elevated">
                <div class="text-xs text-nanna-text-muted">Model</div>
                <div class="text-sm text-nanna-text truncate" :title="selectedAgent.model">
                  {{ formatModel(selectedAgent.model) }}
                </div>
              </div>
              <div class="p-3 rounded-lg bg-nanna-bg-elevated">
                <div class="text-xs text-nanna-text-muted">Tokens In</div>
                <div class="text-sm text-nanna-text font-mono">{{ selectedAgent.tokens_in.toLocaleString() }}</div>
              </div>
              <div class="p-3 rounded-lg bg-nanna-bg-elevated">
                <div class="text-xs text-nanna-text-muted">Tokens Out</div>
                <div class="text-sm text-nanna-text font-mono">{{ selectedAgent.tokens_out.toLocaleString() }}</div>
              </div>
            </div>

            <!-- Workspace -->
            <div v-if="selectedAgent.workspace_path" class="p-3 rounded-lg bg-nanna-bg-elevated">
              <div class="text-xs text-nanna-text-muted mb-1">Workspace</div>
              <div class="text-sm text-nanna-text font-medium">{{ selectedAgent.workspace_name }}</div>
              <div class="text-xs text-nanna-text-dim truncate">{{ selectedAgent.workspace_path }}</div>
            </div>

            <!-- Parent -->
            <div v-if="selectedAgent.parent_id" class="p-3 rounded-lg bg-nanna-bg-elevated">
              <div class="text-xs text-nanna-text-muted mb-1">Parent Agent</div>
              <code class="text-xs text-nanna-accent">{{ selectedAgent.parent_id }}</code>
            </div>

            <!-- Children -->
            <div v-if="selectedAgent.children.length > 0" class="p-3 rounded-lg bg-nanna-bg-elevated">
              <div class="text-xs text-nanna-text-muted mb-2">
                Sub-agents ({{ selectedAgent.children.length }})
              </div>
              <div class="space-y-1">
                <code 
                  v-for="childId in selectedAgent.children" 
                  :key="childId"
                  class="block text-xs text-nanna-text-dim"
                >
                  {{ childId }}
                </code>
              </div>
            </div>

            <!-- Timestamps -->
            <div class="p-3 rounded-lg bg-nanna-bg-elevated">
              <div class="text-xs text-nanna-text-muted mb-2">Timeline</div>
              <div class="space-y-1 text-xs">
                <div class="flex justify-between">
                  <span class="text-nanna-text-dim">Spawned</span>
                  <span class="text-nanna-text">{{ formatTime(selectedAgent.spawned_at) }}</span>
                </div>
                <div class="flex justify-between">
                  <span class="text-nanna-text-dim">Last state change</span>
                  <span class="text-nanna-text">{{ formatTime(selectedAgent.state_changed_at) }}</span>
                </div>
              </div>
            </div>

            <!-- Actions -->
            <div class="flex gap-2 pt-2">
              <UiButton 
                v-if="!isTerminalState(selectedAgent.state)"
                @click="cancelAgent(selectedAgent.id); selectedAgent = null"
                variant="secondary"
                size="sm"
                class="flex-1"
              >
                <StopCircle class="w-4 h-4 mr-1" />
                Cancel
              </UiButton>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { 
  Bot, RefreshCw, FolderOpen, ChevronDown, X, StopCircle, 
  Activity, Clock, Cpu, Zap
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

interface WorkspaceCluster {
  path: string
  name: string
  agents: AgentInfo[]
  total_agents: number
  active_agents: number
  total_tokens_in: number
  total_tokens_out: number
}

interface AgentStats {
  total_agents: number
  active_agents: number
  by_state: Record<string, number>
  by_role: Record<string, number>
  total_tokens_in: number
  total_tokens_out: number
  workspaces: number
}

interface AgentEvent {
  event_type: string
  agent_id: string
  timestamp: number
  data: any
}

const clusters = ref<WorkspaceCluster[]>([])
const stats = ref<AgentStats | null>(null)
const isLoading = ref(false)
const selectedAgent = ref<AgentInfo | null>(null)
const collapsedClusters = ref<Record<string, boolean>>({})

let eventUnlisten: UnlistenFn | null = null

const stateColors: Record<string, string> = {
  spawned: 'bg-nanna-text-dim',
  idle: 'bg-nanna-primary/50',
  thinking: 'bg-nanna-warning',
  tool_use: 'bg-nanna-accent',
  waiting: 'bg-nanna-primary',
  completed: 'bg-nanna-success',
  error: 'bg-nanna-error',
  cancelled: 'bg-nanna-text-dim/50',
}

onMounted(async () => {
  await refreshAgents()
  await subscribeToEvents()
})

onUnmounted(() => {
  if (eventUnlisten) {
    eventUnlisten()
  }
})

async function refreshAgents() {
  isLoading.value = true
  try {
    const [clustersResult, statsResult] = await Promise.all([
      invoke<WorkspaceCluster[]>('get_agent_clusters'),
      invoke<AgentStats>('get_agent_stats'),
    ])
    clusters.value = clustersResult
    stats.value = statsResult
  } catch (e) {
    console.error('Failed to load agents:', e)
  } finally {
    isLoading.value = false
  }
}

async function subscribeToEvents() {
  try {
    await invoke('subscribe_agent_events')
    
    eventUnlisten = await listen<AgentEvent>('agent-event', (event) => {
      console.log('Agent event:', event.payload)
      // Auto-refresh on significant events
      if (['spawned', 'completed', 'error', 'cancelled'].includes(event.payload.event_type)) {
        refreshAgents()
      } else {
        // For state changes, do a lighter update
        updateAgentState(event.payload)
      }
    })
  } catch (e) {
    console.error('Failed to subscribe to agent events:', e)
  }
}

function updateAgentState(event: AgentEvent) {
  // Find and update the agent in clusters
  for (const cluster of clusters.value) {
    const agent = cluster.agents.find(a => a.id === event.agent_id)
    if (agent) {
      if (event.event_type === 'state_changed') {
        agent.state = event.data.new_state
        agent.current_tool = event.data.tool_name || null
        agent.state_changed_at = event.timestamp
      } else if (event.event_type === 'tool_started') {
        agent.current_tool = event.data.tool_name
      } else if (event.event_type === 'tool_completed') {
        agent.current_tool = null
      }
      break
    }
  }
}

function toggleCluster(path: string) {
  collapsedClusters.value[path] = !collapsedClusters.value[path]
}

function selectAgent(agent: AgentInfo) {
  selectedAgent.value = agent
}

async function cancelAgent(agentId: string) {
  try {
    await invoke('cancel_agent', { agentId, reason: 'User cancelled' })
    await refreshAgents()
  } catch (e) {
    console.error('Failed to cancel agent:', e)
  }
}

function getAgentDepth(agent: AgentInfo, allAgents: AgentInfo[]): number {
  if (!agent.parent_id) return 0
  const parent = allAgents.find(a => a.id === agent.parent_id)
  if (!parent) return 0
  return 1 + getAgentDepth(parent, allAgents)
}

function getStateBackground(state: string): string {
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
  return map[state] || 'bg-nanna-bg-elevated'
}

function getStateTextColor(state: string): string {
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
  return map[state] || 'text-nanna-text'
}

function getStateBadge(state: string): string {
  const map: Record<string, string> = {
    spawned: 'bg-nanna-bg-elevated text-nanna-text-muted',
    idle: 'bg-nanna-primary/20 text-nanna-primary',
    thinking: 'bg-nanna-warning/20 text-nanna-warning',
    tool_use: 'bg-nanna-accent/20 text-nanna-accent',
    waiting: 'bg-nanna-primary/20 text-nanna-primary',
    completed: 'bg-nanna-success/20 text-nanna-success',
    error: 'bg-nanna-error/20 text-nanna-error',
    cancelled: 'bg-nanna-text-dim/20 text-nanna-text-dim',
  }
  return map[state] || 'bg-nanna-bg-elevated text-nanna-text'
}

function isTerminalState(state: string): boolean {
  return ['completed', 'error', 'cancelled'].includes(state)
}

function formatTokens(tokens: number): string {
  if (tokens >= 1000000) return `${(tokens / 1000000).toFixed(1)}M`
  if (tokens >= 1000) return `${(tokens / 1000).toFixed(1)}K`
  return tokens.toString()
}

function formatModel(model: string): string {
  // Shorten common model names
  return model
    .replace('claude-', '')
    .replace('-20250514', '')
    .replace('-20241022', '')
    .replace('gpt-', '')
}

function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000)
  return date.toLocaleTimeString()
}
</script>
