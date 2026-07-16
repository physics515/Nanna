<template>
  <div class="space-y-6">
    <!-- System Prompt -->
    <SystemPromptEditor
      @saved="showToast('System prompt saved', 'success')"
      @error="(msg) => showToast(msg, 'error')"
    />

    <!-- Agent Identity -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <Bot class="w-4 h-4" />
        Agent Identity
      </h3>
      <div class="space-y-4">
        <div>
          <label class="block text-sm font-medium text-nanna-text-muted mb-1">Name</label>
          <UiInput v-model="agentName" placeholder="Nanna" @change="saveAgentName" />
          <p class="text-xs text-nanna-text-dim mt-1">The name the agent uses to refer to itself</p>
        </div>

        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Personality Mode</div>
            <div class="text-xs text-nanna-text-dim">How the agent responds</div>
          </div>
          <UiSelect
            v-model="personalityMode"
            @update:model-value="savePersonalityMode"
            :options="[
              { value: 'balanced', label: 'Balanced' },
              { value: 'professional', label: 'Professional' },
              { value: 'casual', label: 'Casual' },
              { value: 'minimal', label: 'Minimal' },
            ]"
            class="w-40"
          />
        </div>
      </div>
    </UiCard>

    <!-- Model Routing -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <Cpu class="w-4 h-4" />
        Model Routing
      </h3>
      <p class="text-xs text-nanna-text-dim mb-4">
        Route simpler tasks to cheaper models automatically. The agent classifies each iteration's complexity
        and picks the cheapest capable model. If the routed model fails, it escalates to the primary model.
      </p>

      <div class="space-y-4">
        <!-- Sub-agent model -->
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Sub-Agent Model</div>
            <div class="text-xs text-nanna-text-dim">Cheaper model for delegated sub-tasks</div>
          </div>
          <UiSelect
            :model-value="subAgentModel || ''"
            @update:model-value="saveSubAgentModel($event)"
            :options="[{ value: '', label: 'Same as primary' }, ...routingModelOptions]"
            class="w-64"
          />
        </div>

        <!-- Enable routing -->
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Enable Routing</div>
            <div class="text-xs text-nanna-text-dim">Use cheaper models for simpler iterations</div>
          </div>
          <UiSwitch :model-value="routingEnabled" @update:model-value="toggleRouting" />
        </div>

        <template v-if="routingEnabled">
          <!-- First turn primary -->
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm font-medium text-nanna-text">Primary on First Turn</div>
              <div class="text-xs text-nanna-text-dim">Always use primary model for the initial response</div>
            </div>
            <UiSwitch :model-value="routingFirstTurnPrimary" @update:model-value="saveRoutingFirstTurnPrimary" />
          </div>

          <!-- Route table -->
          <div class="space-y-3">
            <div class="flex items-center justify-between">
              <label class="text-sm font-medium text-nanna-text">Routes</label>
              <span class="text-xs text-nanna-text-dim">Cheapest first — drag to reorder</span>
            </div>

            <div v-if="modelRoutes.length === 0" class="p-4 rounded-lg bg-nanna-bg-elevated/40 border border-nanna-border/30 text-center">
              <p class="text-xs text-nanna-text-dim">No routes configured. Add a route to start saving on API costs.</p>
            </div>

            <div v-for="(route, index) in modelRoutes" :key="index" class="flex items-center gap-2 p-2 rounded-lg bg-nanna-bg-elevated/40 border border-nanna-border/30">
              <!-- Model select -->
              <UiSelect
                :model-value="route.model"
                @update:model-value="updateRouteModel(index, $event)"
                :options="routingModelOptions"
                placeholder="Select model..."
                class="flex-1"
              />
              <!-- Tier select -->
              <UiSelect
                :model-value="route.tier"
                @update:model-value="updateRouteTier(index, $event)"
                :options="[
                  { value: 'simple', label: '⚡ Simple' },
                  { value: 'medium', label: '⚙️ Medium' },
                  { value: 'complex', label: '🧠 Complex' },
                ]"
                class="w-36"
              />
              <!-- Remove -->
              <button class="p-1.5 rounded hover:bg-nanna-error/20 text-nanna-text-muted hover:text-nanna-error transition-colors" @click="removeRoute(index)">
                <Trash2 class="w-3.5 h-3.5" />
              </button>
            </div>

            <UiButton @click="addRoute" variant="outline" size="sm">
              <Plus class="w-4 h-4 mr-1" />
              Add Route
            </UiButton>
          </div>

          <!-- Complexity guide -->
          <div class="p-3 rounded-lg bg-nanna-bg-elevated/20 border border-nanna-border/20">
            <div class="text-xs font-medium text-nanna-text-muted mb-2">Complexity Tiers</div>
            <div class="space-y-1 text-xs text-nanna-text-dim">
              <div><span class="text-nanna-text">⚡ Simple</span> — tool result processing, acknowledgments, straightforward tool calls</div>
              <div><span class="text-nanna-text">⚙️ Medium</span> — multi-step reasoning, code generation, summarization</div>
              <div><span class="text-nanna-text">🧠 Complex</span> — novel problem solving, long-form analysis, ambiguous requests</div>
            </div>
          </div>
        </template>
      </div>
    </UiCard>

    <!-- Response Preferences -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <MessageSquare class="w-4 h-4" />
        Response Preferences
      </h3>
      <div class="space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Thinking Mode</div>
            <div class="text-xs text-nanna-text-dim">Show reasoning process</div>
          </div>
          <UiSwitch :model-value="settings?.thinking_enabled" @update:model-value="setThinkingEnabled" />
        </div>

        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Streaming</div>
            <div class="text-xs text-nanna-text-dim">Stream responses token by token</div>
          </div>
          <UiSwitch :model-value="settings?.streaming_enabled ?? true" @update:model-value="setStreamingEnabled" />
        </div>

        <!-- Max Tokens -->
        <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm font-medium text-nanna-text">Max Response Length</span>
            <span class="text-sm text-nanna-accent font-mono">{{ maxTokens.toLocaleString() }}</span>
          </div>
          <input
            type="range" min="256" max="8192" step="256"
            :value="maxTokens"
            @change="setMaxTokens(Number(($event.target as HTMLInputElement).value))"
            class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
          />
          <div class="flex justify-between text-xs text-nanna-text-dim mt-1">
            <span>256</span>
            <span>8192 tokens</span>
          </div>
        </div>

        <!-- Agent Loop (long-horizon) -->
        <div class="p-3 rounded-lg bg-nanna-bg-elevated/40 space-y-3">
          <div class="flex items-center justify-between">
            <div>
              <span class="text-sm font-medium text-nanna-text">Agent Loop</span>
              <p class="text-xs text-nanna-text-dim mt-0.5">
                Nanna can work on a problem for many iterations. It never hard-stops —
                Stop always ends a run — and only gets gentle "wrap up" nudges once a run gets long.
              </p>
            </div>
          </div>

          <!-- Unlimited backstop toggle -->
          <label class="flex items-center justify-between cursor-pointer">
            <span class="text-sm text-nanna-text">Unlimited iterations (recommended)</span>
            <input
              type="checkbox"
              v-model="unlimitedIterations"
              @change="saveIterationPolicy"
              class="accent-nanna-primary w-4 h-4"
            />
          </label>

          <!-- Absolute backstop (only when not unlimited) -->
          <div v-if="!unlimitedIterations" class="flex items-center justify-between gap-3">
            <span class="text-sm text-nanna-text-dim">Max iterations (safety backstop)</span>
            <input
              type="number" min="1" step="100"
              v-model.number="maxIterations"
              @change="saveIterationPolicy"
              class="w-28 px-2 py-1 text-sm text-right rounded bg-nanna-bg-deep text-nanna-text border border-nanna-border font-mono"
            />
          </div>

          <!-- First nudge -->
          <div class="flex items-center justify-between gap-3">
            <span class="text-sm text-nanna-text-dim">Nudge after (iterations)</span>
            <input
              type="number" min="1" step="50"
              v-model.number="nudgeAfterIterations"
              @change="saveIterationPolicy"
              class="w-28 px-2 py-1 text-sm text-right rounded bg-nanna-bg-deep text-nanna-text border border-nanna-border font-mono"
            />
          </div>

          <!-- Nudge interval -->
          <div class="flex items-center justify-between gap-3">
            <span class="text-sm text-nanna-text-dim">Then re-nudge every (iterations)</span>
            <input
              type="number" min="1" step="25"
              v-model.number="nudgeIntervalIterations"
              @change="saveIterationPolicy"
              class="w-28 px-2 py-1 text-sm text-right rounded bg-nanna-bg-deep text-nanna-text border border-nanna-border font-mono"
            />
          </div>
        </div>
      </div>
    </UiCard>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Bot, Cpu, MessageSquare, Trash2, Plus } from 'lucide-vue-next'
import { useSettingsPage } from '~/composables/useSettingsPage'

const store = useSettingsPage()
const { settings, routingModelOptions, loadSettings, showToast } = store

const agentName = ref('Nanna')
const personalityMode = ref('balanced')
const maxTokens = ref(4096)
// Agent-loop iteration policy (long-horizon worker).
// unlimitedIterations = true means no absolute backstop (only Stop/cancel ends a run).
const unlimitedIterations = ref(true)
const maxIterations = ref(10000)
const nudgeAfterIterations = ref(500)
const nudgeIntervalIterations = ref(100)

// Model routing state
interface RouteEntry {
  model: string
  tier: string
}
const modelRoutes = ref<RouteEntry[]>([])
const routingEnabled = ref(false)
const routingFirstTurnPrimary = ref(true)
const subAgentModel = ref<string | null>(null)

// Repopulate tab state whenever settings are (re)loaded
store.onSettingsLoaded(async () => {
  if (!settings.value) return
  // Load agent settings
  agentName.value = settings.value.agent_name || 'Nanna'
  personalityMode.value = settings.value.personality_mode || 'balanced'
  maxTokens.value = settings.value.max_tokens || 4096
  // Agent-loop iteration policy
  const maxIter = settings.value.agent_max_iterations
  unlimitedIterations.value = maxIter === null || maxIter === undefined
  if (typeof maxIter === 'number') maxIterations.value = maxIter
  nudgeAfterIterations.value = settings.value.agent_nudge_after_iterations ?? 500
  nudgeIntervalIterations.value = settings.value.agent_nudge_interval_iterations ?? 100

  // Load model routing config
  try {
    const routes = await invoke<string[]>('get_model_routing')
    modelRoutes.value = routes.map(parseRouteSpec)
    routingEnabled.value = routes.length > 0
  } catch {
    modelRoutes.value = []
    routingEnabled.value = false
  }
  try {
    routingFirstTurnPrimary.value = await invoke<boolean>('get_routing_first_turn_primary')
  } catch {
    routingFirstTurnPrimary.value = true
  }
  try {
    subAgentModel.value = await invoke<string | null>('get_sub_agent_model')
  } catch {
    subAgentModel.value = null
  }
})

// ── Model Routing ──

function parseRouteSpec(spec: string): RouteEntry {
  // Parse "model:tier" — but handle model names with colons (e.g. "ollama/qwen3:4b")
  // Tier is always the last segment and must be simple|medium|complex
  const lastColon = spec.lastIndexOf(':')
  if (lastColon > 0) {
    const maybeTier = spec.slice(lastColon + 1).toLowerCase()
    if (['simple', 'medium', 'complex'].includes(maybeTier)) {
      return { model: spec.slice(0, lastColon), tier: maybeTier }
    }
  }
  return { model: spec, tier: 'complex' }
}

function serializeRoutes(): string[] {
  return modelRoutes.value
    .filter(r => r.model)
    .map(r => `${r.model}:${r.tier}`)
}

async function saveRoutes() {
  try {
    await invoke('set_model_routing', { routes: serializeRoutes() })
  } catch (e: any) {
    showToast(`Failed to save routing: ${e.message || e}`, 'error')
  }
}

function toggleRouting(enabled: boolean) {
  routingEnabled.value = enabled
  if (!enabled) {
    // Clear routes when disabled
    modelRoutes.value = []
    saveRoutes()
  }
}

async function saveRoutingFirstTurnPrimary(enabled: boolean) {
  routingFirstTurnPrimary.value = enabled
  try {
    await invoke('set_routing_first_turn_primary', { enabled })
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

function addRoute() {
  modelRoutes.value.push({ model: '', tier: 'simple' })
}

function removeRoute(index: number) {
  modelRoutes.value.splice(index, 1)
  saveRoutes()
}

function updateRouteModel(index: number, model: string) {
  modelRoutes.value[index].model = model
  saveRoutes()
}

function updateRouteTier(index: number, tier: string) {
  modelRoutes.value[index].tier = tier
  saveRoutes()
}

async function saveSubAgentModel(model: string) {
  const value = model || null
  subAgentModel.value = value
  try {
    await invoke('set_sub_agent_model', { model: value })
    showToast(value ? `Sub-agent model: ${value}` : 'Sub-agents will use primary model', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveAgentName() {
  try {
    await invoke('set_agent_name', { name: agentName.value })
    showToast('Agent name saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function savePersonalityMode() {
  try {
    await invoke('set_personality_mode', { mode: personalityMode.value })
    showToast('Personality mode saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setThinkingEnabled(enabled: boolean) {
  try {
    await invoke('set_thinking_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setStreamingEnabled(enabled: boolean) {
  try {
    await invoke('set_streaming_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setMaxTokens(tokens: number) {
  try {
    await invoke('set_max_tokens', { tokens })
    maxTokens.value = tokens
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveIterationPolicy() {
  try {
    await invoke('set_agent_iteration_policy', {
      maxIterations: unlimitedIterations.value ? null : Math.max(1, Math.round(maxIterations.value)),
      nudgeAfter: Math.max(1, Math.round(nudgeAfterIterations.value)),
      nudgeInterval: Math.max(1, Math.round(nudgeIntervalIterations.value)),
    })
    showToast('Agent loop settings saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}
</script>
