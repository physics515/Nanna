<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04] bg-nanna-bg-surface/80">
      <div class="flex items-center gap-3 sm:gap-4">
        <NuxtLink to="/" class="text-nanna-text-muted hover:text-nanna-text transition-colors">
          <ArrowLeft class="w-5 h-5" />
        </NuxtLink>
        <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Settings</h2>
        <div class="ml-auto flex gap-2">
          <UiButton v-if="hasChanges" @click="saveAllSettings" size="sm" :disabled="saving">
            <Save class="w-4 h-4 mr-1" />
            {{ saving ? 'Saving...' : 'Save' }}
          </UiButton>
        </div>
      </div>
    </header>
    
    <!-- Tabs -->
    <div class="px-4 sm:px-6 pt-4">
      <UiTabs v-model="activeTab" :tabs="tabs" />
    </div>
    
    <!-- Tab Content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <div class="max-w-2xl mx-auto">
        
        <!-- Models Tab -->
        <UiTabPanel :active="activeTab === 'models'">
          <div class="space-y-6">
            <!-- API Keys -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
                <Key class="w-4 h-4" />
                Providers
              </h3>
              <div class="space-y-4">
                <!-- Anthropic (OAuth only, via claude setup-token) -->
                <div class="space-y-3">
                  <div class="flex items-center justify-between">
                    <label class="text-sm font-medium text-nanna-text">Anthropic</label>
                    <UiBadge v-if="settings?.anthropic_oauth_logged_in" variant="success">Logged In</UiBadge>
                  </div>

                  <!-- Logged in state -->
                  <div v-if="settings?.anthropic_oauth_logged_in" class="space-y-3">
                    <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-success/10 border border-nanna-success/30">
                      <div class="flex items-center gap-2">
                        <CheckCircle class="w-4 h-4 text-nanna-success" />
                        <span class="text-sm text-nanna-text">Authenticated via Claude Code</span>
                      </div>
                      <UiButton @click="logoutAnthropic" variant="ghost" size="sm">
                        <LogOut class="w-4 h-4 mr-1" />
                        Logout
                      </UiButton>
                    </div>
                  </div>

                  <!-- Not logged in state -->
                  <div v-else class="space-y-3">
                    <p class="text-xs text-nanna-text-muted">
                      Run <code class="bg-nanna-bg-elevated px-1 rounded">claude setup-token</code> and paste the token below.
                    </p>

                    <!-- Token input box -->
                    <div class="flex gap-2">
                      <UiInput
                        v-model="oauthTokenInput"
                        type="password"
                        placeholder="Paste token from claude setup-token..."
                        class="flex-1 font-mono text-sm"
                      />
                      <UiButton @click="saveOAuthToken" :disabled="!oauthTokenInput.trim() || oauthLoading" size="sm">
                        <UiSpinner v-if="oauthLoading" size="sm" class="mr-1" />
                        {{ oauthLoading ? 'Saving...' : 'Save' }}
                      </UiButton>
                    </div>

                    <!-- Helper buttons -->
                    <div class="flex gap-2">
                      <UiButton @click="runClaudeSetupToken" :disabled="oauthLoading" variant="outline" size="sm" class="flex-1">
                        <UiSpinner v-if="oauthLoading && oauthAction === 'setup'" size="sm" class="mr-1" />
                        <Terminal v-else class="w-3 h-3 mr-1" />
                        {{ oauthLoading && oauthAction === 'setup' ? 'Running...' : 'Run CLI' }}
                      </UiButton>
                      <UiButton @click="importClaudeCodeCredentials" :disabled="oauthLoading" variant="outline" size="sm" class="flex-1">
                        <UiSpinner v-if="oauthLoading && oauthAction === 'import'" size="sm" class="mr-1" />
                        <Download v-else class="w-3 h-3 mr-1" />
                        {{ oauthLoading && oauthAction === 'import' ? 'Importing...' : 'Import' }}
                      </UiButton>
                    </div>
                  </div>
                </div>

                <ApiKeyInput
                  label="OpenAI"
                  provider="openai"
                  placeholder="sk-..."
                  :is-set="settings?.openai_key_set"
                  hint="For GPT models and embeddings"
                  @save="saveApiKey"
                />
                <ApiKeyInput
                  label="OpenRouter"
                  provider="openrouter"
                  placeholder="sk-or-..."
                  :is-set="settings?.openrouter_key_set"
                  hint="For multi-provider access"
                  @save="saveApiKey"
                />
                <ApiKeyInput
                  label="GitHub Models"
                  provider="github"
                  placeholder="ghp_..."
                  :is-set="settings?.github_key_set"
                  hint="Use Copilot models via GitHub token"
                  @save="saveApiKey"
                />

                <!-- Claude Proxy (claude-max-api-proxy) -->
                <div class="space-y-2 p-3 rounded-lg bg-nanna-bg-elevated/40 border border-nanna-border/30">
                  <div class="flex items-center justify-between">
                    <div class="flex items-center gap-2">
                      <span class="text-sm font-medium text-nanna-text">Claude Proxy</span>
                      <span
                        v-if="claudeProxyHealthy"
                        class="px-1.5 py-0.5 text-[10px] rounded bg-nanna-success/20 text-nanna-success"
                      >Online</span>
                      <span
                        v-else-if="settings?.claude_proxy_enabled"
                        class="px-1.5 py-0.5 text-[10px] rounded bg-nanna-error/20 text-nanna-error"
                      >Offline</span>
                    </div>
                    <label class="relative inline-flex items-center cursor-pointer">
                      <input
                        type="checkbox"
                        :checked="settings?.claude_proxy_enabled"
                        @change="toggleClaudeProxy"
                        class="sr-only peer"
                      >
                      <div class="w-9 h-5 bg-nanna-bg-elevated peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-nanna-text-muted after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-nanna-primary peer-checked:after:bg-white"></div>
                    </label>
                  </div>
                  <p class="text-xs text-nanna-text-muted">
                    Route through claude-max-api-proxy to use Claude Pro/Max subscription
                  </p>
                  <div v-if="settings?.claude_proxy_enabled" class="flex gap-2 mt-2">
                    <input
                      v-model="claudeProxyUrl"
                      type="text"
                      placeholder="http://localhost:3456"
                      class="flex-1 px-2 py-1.5 text-sm bg-nanna-bg rounded border border-nanna-border/50 text-nanna-text placeholder:text-nanna-text-muted/50 focus:outline-none focus:border-nanna-primary"
                    >
                    <UiButton @click="saveClaudeProxyUrl" size="sm" variant="ghost">Save</UiButton>
                  </div>
                </div>

                <!-- Ollama -->
                <div class="space-y-3 p-3 rounded-lg bg-nanna-bg-elevated/40 border border-nanna-border/30">
                  <div class="flex items-center justify-between">
                    <span class="text-sm font-medium text-nanna-text">Ollama</span>
                    <div class="flex items-center gap-2">
                      <span v-if="ollamaStatus === 'connected'" class="flex items-center gap-1 text-xs text-green-500">
                        <CheckCircle class="w-3 h-3" /> {{ ollamaModels.length }} model{{ ollamaModels.length !== 1 ? 's' : '' }}
                      </span>
                      <span v-else-if="ollamaStatus === 'error'" class="flex items-center gap-1 text-xs text-red-400">
                        <XCircle class="w-3 h-3" /> Offline
                      </span>
                      <UiButton @click="refreshOllamaModels" size="sm" variant="ghost" :disabled="loadingOllamaModels">
                        <RefreshCw class="w-3 h-3" :class="{ 'animate-spin': loadingOllamaModels }" />
                      </UiButton>
                    </div>
                  </div>
                  <div>
                    <label class="block text-xs text-nanna-text-dim mb-1">Server URL</label>
                    <div class="flex gap-2">
                      <UiInput v-model="ollamaHostInput" placeholder="http://localhost:11434" class="flex-1" />
                      <UiButton @click="saveOllamaHost" size="sm">Save</UiButton>
                    </div>
                  </div>
                  <div>
                    <label class="block text-xs text-nanna-text-dim mb-1">API Key <span class="text-nanna-text-dim/60">(optional)</span></label>
                    <div class="flex gap-2">
                      <UiInput v-model="ollamaApiKeyInput" type="password" placeholder="For remote/authenticated instances" class="flex-1" />
                      <UiButton @click="saveOllamaApiKey" size="sm">Save</UiButton>
                    </div>
                  </div>
                </div>
              </div>
            </UiCard>

            <!-- Model Priority (Fallback Chain) -->
            <UiCard>
              <div class="flex items-center justify-between mb-4">
                <h3 class="text-base font-semibold text-nanna-primary flex items-center gap-2">
                  <Brain class="w-4 h-4" />
                  Chat Models
                </h3>
                <UiButton @click="refreshAllModels" :disabled="loadingModels" variant="ghost" size="sm">
                  <RefreshCw :class="['w-3 h-3', loadingModels && 'animate-spin']" />
                </UiButton>
              </div>
              
              <ModelPriorityList
                label="Model Priority"
                hint="First working model is used. Drag to reorder fallback priority."
                :all-models="allChatModels"
                v-model="chatModelPriority"
                @update:model-value="saveChatModelPriority"
              />
              
              <!-- No Models Warning -->
              <div v-if="allChatModels.length === 0" class="mt-4 p-4 rounded-lg bg-nanna-warning/10 border border-nanna-warning/30">
                <div class="flex items-start gap-3">
                  <AlertTriangle class="w-5 h-5 text-nanna-warning shrink-0 mt-0.5" />
                  <div>
                    <div class="font-medium text-nanna-warning">No models available</div>
                    <p class="text-sm text-nanna-text-muted mt-1">
                      Set up an API key below, or configure Ollama for local models.
                    </p>
                  </div>
                </div>
              </div>

              <!-- Summarization Models -->
              <div class="mt-6 pt-6 border-t border-white/[0.04]">
                <h3 class="text-sm font-semibold text-nanna-primary mb-3 flex items-center gap-2">
                  <Layers class="w-4 h-4" />
                  Context Summarization
                </h3>
                <p class="text-xs text-nanna-text-dim mb-3">
                  When conversation context exceeds the chat model's limit, these models recursively summarize older content until it fits. Any chat model works; local Ollama models avoid API costs.
                </p>
                <ModelPriorityList
                  label="Summarization Model Priority"
                  hint="Used to compress context when it exceeds limits. Empty = truncate instead of summarize."
                  :all-models="allSummarizationModels"
                  v-model="summarizationModelPriority"
                  @update:model-value="saveSummarizationModelPriority"
                />
              </div>

              <!-- Embedding Models -->
              <div class="mt-6 pt-6 border-t border-white/[0.04]">
                <h3 class="text-sm font-semibold text-nanna-primary mb-3 flex items-center gap-2">
                  <Link class="w-4 h-4" />
                  Embedding Models
                </h3>
                <p class="text-xs text-nanna-text-dim mb-3">
                  Used for semantic memory recall. First working model is used.
                </p>
                <ModelPriorityList
                  label="Embedding Priority"
                  hint="Used for memory recall. First working model is used."
                  :all-models="allEmbeddingModels"
                  v-model="embeddingModelPriority"
                  @update:model-value="saveEmbeddingModelPriority"
                />
                <div v-if="allEmbeddingModels.length === 0" class="mt-3 p-3 rounded-lg bg-nanna-warning/10 border border-nanna-warning/30">
                  <div class="flex items-start gap-2">
                    <AlertTriangle class="w-4 h-4 text-nanna-warning shrink-0 mt-0.5" />
                    <p class="text-xs text-nanna-warning">No embedding models available. Set up an API key or install Ollama embedding models.</p>
                  </div>
                </div>
                <div v-else class="flex items-center gap-2 mt-3">
                  <UiBadge v-if="embeddingModelPriority.length > 0" variant="success">✓ Memory recall enabled</UiBadge>
                  <UiBadge v-else variant="warning">⚠ No embedding models selected — memory recall disabled</UiBadge>
                </div>
              </div>

              <!-- OCR Models -->
              <div class="mt-6 pt-6 border-t border-white/[0.04]">
                <h3 class="text-sm font-semibold text-nanna-primary mb-3 flex items-center gap-2">
                  <ScanText class="w-4 h-4" />
                  OCR Models
                </h3>
                <p class="text-xs text-nanna-text-dim mb-3">
                  Used to extract text from images and scanned PDFs. Tier 0 is the built-in <code>ocrs</code> engine (offline, no API cost). Tier 1+ are vision-capable models tried in order.
                </p>

                <!-- Embedded OCR toggle -->
                <div class="flex items-center justify-between mb-4 p-3 rounded-lg bg-white/[0.02] border border-white/[0.05]">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Use embedded OCR first (Tier 0)</div>
                    <div class="text-xs text-nanna-text-dim mt-0.5">Runs offline using the <code>ocrs</code> ONNX engine — models auto-downloaded to <code>~/.cache/ocrs/</code> on first use (~50 MB). Latin script only.</div>
                  </div>
                  <UiSwitch
                    :model-value="useEmbeddedOcr"
                    @update:model-value="saveUseEmbeddedOcr"
                  />
                </div>

                <!-- Vision model fallback list -->
                <ModelPriorityList
                  label="Vision Model Fallback (Tier 1+)"
                  hint="Only vision-capable models shown. Used when embedded OCR fails or returns no text."
                  :all-models="allOcrModels"
                  v-model="ocrModelPriority"
                  @update:model-value="saveOcrModelPriority"
                />

                <div v-if="allOcrModels.length === 0" class="mt-3 p-3 rounded-lg bg-nanna-warning/10 border border-nanna-warning/30">
                  <div class="flex items-start gap-2">
                    <AlertTriangle class="w-4 h-4 text-nanna-warning shrink-0 mt-0.5" />
                    <p class="text-xs text-nanna-warning">No vision-capable models available. Install a vision Ollama model (e.g. llava) or set up an Anthropic/OpenAI API key.</p>
                  </div>
                </div>
                <div v-else class="flex items-center gap-2 mt-3">
                  <UiBadge v-if="useEmbeddedOcr" variant="success">✓ Embedded OCR active</UiBadge>
                  <UiBadge v-if="ocrModelPriority.length > 0" variant="success">✓ {{ ocrModelPriority.length }} vision model{{ ocrModelPriority.length !== 1 ? 's' : '' }} in fallback chain</UiBadge>
                  <UiBadge v-if="!useEmbeddedOcr && ocrModelPriority.length === 0" variant="warning">⚠ No OCR methods configured</UiBadge>
                </div>
              </div>

            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Agent Tab -->
        <UiTabPanel :active="activeTab === 'agent'">
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
        </UiTabPanel>

        <!-- Memory Tab -->
        <UiTabPanel :active="activeTab === 'memory'">
          <div class="space-y-6">
            <!-- Memory Settings -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
                <BrainCircuit class="w-4 h-4" />
                Cognitive Memory
                <UiBadge variant="secondary" class="ml-auto">FSRS-6</UiBadge>
              </h3>
              <div class="space-y-4">
                <!-- Stats Grid -->
                <div class="grid grid-cols-4 gap-2">
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
                    <div class="text-lg font-bold text-nanna-success">{{ memoryStats?.active || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Active</div>
                  </div>
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
                    <div class="text-lg font-bold text-nanna-warning">{{ memoryStats?.dormant || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Dormant</div>
                  </div>
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
                    <div class="text-lg font-bold text-nanna-text-muted">{{ memoryStats?.silent || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Silent</div>
                  </div>
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
                    <div class="text-lg font-bold text-nanna-error">{{ memoryStats?.unavailable || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Faded</div>
                  </div>
                </div>
                
                <!-- Similarity Threshold -->
                <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-sm font-medium text-nanna-text">Recall Threshold</span>
                    <span class="text-sm text-nanna-accent font-mono">{{ (similarityThreshold * 100).toFixed(0) }}%</span>
                  </div>
                  <input 
                    type="range" min="0" max="100" step="5"
                    :value="similarityThreshold * 100"
                    @change="setSimilarityThreshold(Number(($event.target as HTMLInputElement).value) / 100)"
                    class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
                  >
                  <p class="text-xs text-nanna-text-dim mt-1">Lower = more results, higher = more precise</p>
                </div>
                
                <!-- Toggles -->
                <div class="space-y-3">
                  <div class="flex items-center justify-between">
                    <div>
                      <div class="text-sm font-medium text-nanna-text">Enable Dreaming</div>
                      <div class="text-xs text-nanna-text-dim">Memory consolidation</div>
                    </div>
                    <UiSwitch :model-value="settings?.dreaming_enabled" @update:model-value="setDreamingEnabled" />
                  </div>
                </div>
                
                <!-- Consolidation Settings -->
                <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-sm font-medium text-nanna-text">Max Compression</span>
                    <span class="text-sm text-nanna-accent font-mono">{{ ((settings?.max_compression_ratio ?? 0.5) * 100).toFixed(0) }}%</span>
                  </div>
                  <input 
                    type="range" min="10" max="90" step="5"
                    :value="(settings?.max_compression_ratio ?? 0.5) * 100"
                    @change="setMaxCompressionRatio(Number(($event.target as HTMLInputElement).value) / 100)"
                    class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
                  >
                  <p class="text-xs text-nanna-text-dim mt-1">Max fraction of memories that can be merged per consolidation run</p>
                </div>
                <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-sm font-medium text-nanna-text">Min Memories Floor</span>
                    <span class="text-sm text-nanna-accent font-mono">{{ settings?.min_remaining_memories ?? 20 }}</span>
                  </div>
                  <input 
                    type="range" min="5" max="200" step="5"
                    :value="settings?.min_remaining_memories ?? 20"
                    @change="setMinRemainingMemories(Number(($event.target as HTMLInputElement).value))"
                    class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
                  >
                  <p class="text-xs text-nanna-text-dim mt-1">Never consolidate below this many memories</p>
                </div>

                <!-- Dream Button -->
                <UiButton @click="triggerConsolidation" :disabled="consolidating || !settings?.dreaming_enabled" class="w-full">
                  <UiSpinner v-if="consolidating" size="sm" class="mr-2" />
                  <Moon v-else class="w-4 h-4 mr-2" />
                  {{ consolidating ? 'Dreaming...' : 'Dream Now' }}
                </UiButton>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Tools Tab -->
        <UiTabPanel :active="activeTab === 'tools'">
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
        </UiTabPanel>
        
        <!-- Scheduler Tab -->
        <UiTabPanel :active="activeTab === 'scheduler'">
          <div class="space-y-6">
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
                <Clock class="w-4 h-4" />
                Scheduler Settings
              </h3>
              <div class="space-y-4">
                <div class="flex items-center justify-between">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Enable Scheduler</div>
                    <div class="text-xs text-nanna-text-dim">Background tasks</div>
                  </div>
                  <UiSwitch :model-value="settings?.scheduler_enabled" @update:model-value="setSchedulerEnabled" />
                </div>
                
                <div class="flex items-center justify-between">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Enable Heartbeats</div>
                    <div class="text-xs text-nanna-text-dim">Periodic self-checks</div>
                  </div>
                  <UiSwitch :model-value="settings?.heartbeat_enabled" @update:model-value="setHeartbeatEnabled" />
                </div>
                
                <!-- Heartbeat Interval -->
                <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-sm font-medium text-nanna-text">Heartbeat Interval</span>
                    <span class="text-sm text-nanna-accent font-mono">{{ formatInterval(settings?.heartbeat_interval_seconds || 300) }}</span>
                  </div>
                  <input 
                    type="range" min="60" max="1800" step="60"
                    :value="settings?.heartbeat_interval_seconds || 300"
                    @change="setHeartbeatInterval(Number(($event.target as HTMLInputElement).value))"
                    class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
                  >
                  <div class="flex justify-between text-xs text-nanna-text-dim mt-1">
                    <span>1 min</span>
                    <span>30 min</span>
                  </div>
                </div>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Data Tab -->
        <UiTabPanel :active="activeTab === 'data'">
          <div class="space-y-6">
            <!-- Sessions -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
                <Database class="w-4 h-4" />
                Data Management
              </h3>
              <div class="space-y-4">
                <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/40">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Chat Sessions</div>
                    <div class="text-xs text-nanna-text-dim">{{ sessionCount }} sessions stored</div>
                  </div>
                  <UiButton @click="confirmClearSessions" variant="destructive" size="sm">
                    <Trash2 class="w-4 h-4 mr-1" />
                    Clear All
                  </UiButton>
                </div>
                
                <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/40">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Memories</div>
                    <div class="text-xs text-nanna-text-dim">{{ memoryStats?.total_memories || 0 }} memories stored</div>
                  </div>
                  <UiButton @click="confirmClearMemories" variant="destructive" size="sm">
                    <Trash2 class="w-4 h-4 mr-1" />
                    Clear All
                  </UiButton>
                </div>
              </div>
            </UiCard>
            
            <!-- Import/Export -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
                <FileDown class="w-4 h-4" />
                Configuration
              </h3>
              <div class="space-y-3">
                <p class="text-sm text-nanna-text-muted">
                  Config file location:
                </p>
                <code class="block text-xs bg-nanna-bg-deep text-nanna-accent p-2 rounded font-mono break-all">
                  {{ configPath }}
                </code>
                <div class="flex gap-2">
                  <UiButton @click="exportConfig" variant="secondary" size="sm" class="flex-1">
                    <FileDown class="w-4 h-4 mr-1" />
                    Export
                  </UiButton>
                  <UiButton @click="importConfig" variant="secondary" size="sm" class="flex-1">
                    <FileUp class="w-4 h-4 mr-1" />
                    Import
                  </UiButton>
                </div>
              </div>
            </UiCard>
            
            <!-- About -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
                <Moon class="w-4 h-4" />
                About Nanna
              </h3>
              <p class="text-sm text-nanna-text-muted italic mb-3">
                "I am the light that finds you in darkness, the memory that outlives the flesh."
              </p>
              <div class="space-y-2 text-sm">
                <div class="flex justify-between">
                  <span class="text-nanna-text-muted">Version</span>
                  <span class="text-nanna-text font-mono">0.1.0</span>
                </div>
                <div class="flex justify-between">
                  <span class="text-nanna-text-muted">Stack</span>
                  <span class="text-nanna-text">Tauri v2 + Nuxt v4 + Rust</span>
                </div>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
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
import { useConfirm } from '~/composables/useConfirm'
import {
  ArrowLeft, Key, Brain, Link, Wrench, BrainCircuit, Database, Moon,
  RefreshCw, Trash2, CheckCircle, XCircle, Save, Clock, FileDown, FileUp,
  Bot, MessageSquare, AlertTriangle, LogOut, Download, Terminal, Layers,
  Cpu, Plus, ScanText
} from 'lucide-vue-next'

interface ToolInfo {
  name: string
  description: string
  enabled: boolean
}

interface ExtendedSettings {
  anthropic_key_set: boolean
  openai_key_set: boolean
  openrouter_key_set: boolean
  github_key_set: boolean
  claude_proxy_enabled: boolean
  claude_proxy_url: string
  brave_key_set: boolean
  // Anthropic OAuth
  anthropic_oauth_logged_in: boolean
  anthropic_use_oauth: boolean
  provider: string
  available_providers: string[]
  model: string
  embedding_provider: string
  embedding_model: string
  embedding_enabled: boolean
  ollama_host: string
  ollama_api_key: string
  tools: ToolInfo[]
  dreaming_enabled: boolean
  max_compression_ratio: number
  min_remaining_memories: number
  scheduler_enabled: boolean
  heartbeat_enabled: boolean
  heartbeat_interval_seconds: number
  thinking_enabled?: boolean
  streaming_enabled?: boolean
  max_tokens?: number
  agent_name?: string
  personality_mode?: string
  agent_max_iterations?: number | null
  agent_nudge_after_iterations?: number
  agent_nudge_interval_iterations?: number
}

interface CognitiveMemoryStats {
  total_memories: number
  active: number
  dormant: number
  silent: number
  unavailable: number
}

interface ModelInfo {
  id: string
  name: string
}

interface OllamaModelInfo {
  name: string
  size_mb: number
  is_embedding_model: boolean
}

const tabs = [
  { id: 'models', label: 'Models', icon: Brain },
  { id: 'agent', label: 'Agent', icon: Bot },
  { id: 'memory', label: 'Memory', icon: BrainCircuit },
  { id: 'tools', label: 'Tools', icon: Wrench },
  { id: 'scheduler', label: 'Scheduler', icon: Clock },
  { id: 'data', label: 'Data', icon: Database },
]

const { confirm } = useConfirm()

const activeTab = ref('models')
const settings = ref<ExtendedSettings | null>(null)
const selectedModel = ref('')
const selectedEmbeddingModel = ref('')
const sessionCount = ref(0)
const ollamaModels = ref<OllamaModelInfo[]>([])
const loadingOllamaModels = ref(false)
const ollamaHostInput = ref('')
const ollamaApiKeyInput = ref('')
const ollamaStatus = ref<'unchecked' | 'checking' | 'connected' | 'error'>('unchecked')
const ollamaError = ref('')
const availableModels = ref<ModelInfo[]>([])
const loadingModels = ref(false)
const memoryStats = ref<CognitiveMemoryStats | null>(null)
const consolidating = ref(false)
const toast = ref<{ message: string; type: 'success' | 'error' } | null>(null)
const similarityThreshold = ref(0.4)
const hasChanges = ref(false)
const saving = ref(false)
const agentName = ref('Nanna')
const personalityMode = ref('balanced')
const maxTokens = ref(4096)
// Agent-loop iteration policy (long-horizon worker).
// unlimitedIterations = true means no absolute backstop (only Stop/cancel ends a run).
const unlimitedIterations = ref(true)
const maxIterations = ref(10000)
const nudgeAfterIterations = ref(500)
const nudgeIntervalIterations = ref(100)

// Model priority lists (fallback chains)
const chatModelPriority = ref<string[]>([])
const embeddingModelPriority = ref<string[]>([])
const summarizationModelPriority = ref<string[]>([])

// OCR settings
const ocrModelPriority = ref<string[]>([])
const useEmbeddedOcr = ref(true)

// Model routing state
interface RouteEntry {
  model: string
  tier: string
}
const modelRoutes = ref<RouteEntry[]>([])
const routingEnabled = ref(false)
const routingFirstTurnPrimary = ref(true)
const subAgentModel = ref<string | null>(null)

// Anthropic OAuth state
const oauthTokenInput = ref('')
const oauthLoading = ref(false)
const oauthAction = ref<'setup' | 'import' | null>(null)

// Dynamically fetched models from APIs
const anthropicModels = ref<ModelInfo[]>([])
const openaiModels = ref<ModelInfo[]>([])
const openrouterModels = ref<ModelInfo[]>([])
const openrouterEmbeddingModels = ref<ModelInfo[]>([])
const githubModels = ref<ModelInfo[]>([])
const claudeProxyModels = ref<ModelInfo[]>([])

// Claude Proxy state
const claudeProxyUrl = ref('http://localhost:3456')
const claudeProxyHealthy = ref(false)

// All available models with provider info
import type { ModelOption } from '~/components/ModelPriorityList.vue'

const allChatModels = computed<ModelOption[]>(() => {
  const models: ModelOption[] = []

  // Anthropic models (dynamically fetched from API)
  // Available if either API key is set or OAuth is logged in
  const anthropicAvailable = settings.value?.anthropic_key_set || settings.value?.anthropic_oauth_logged_in
  if (anthropicAvailable && anthropicModels.value.length > 0) {
    for (const m of anthropicModels.value) {
      models.push({ id: m.id, name: m.name, provider: 'anthropic', available: true })
    }
  }

  // OpenAI models (dynamically fetched from API)
  if (settings.value?.openai_key_set && openaiModels.value.length > 0) {
    const chatModels = openaiModels.value.filter(m =>
      m.id.startsWith('gpt-') || m.id.startsWith('o1') || m.id.startsWith('o3') || m.id.startsWith('chatgpt')
    )
    for (const m of chatModels) {
      models.push({ id: m.id, name: m.name, provider: 'openai', available: true })
    }
  }

  // OpenRouter models (dynamically fetched from API)
  // Prefix with openrouter/ so parse_model_id recognizes the provider
  if (settings.value?.openrouter_key_set && openrouterModels.value.length > 0) {
    for (const m of openrouterModels.value) {
      models.push({ id: `openrouter/${m.id}`, name: m.name, provider: 'openrouter', available: true })
    }
  }

  // GitHub Models (dynamically fetched from API)
  if (settings.value?.github_key_set && githubModels.value.length > 0) {
    for (const m of githubModels.value) {
      models.push({ id: `github/${m.id}`, name: m.name, provider: 'github', available: true })
    }
  }

  // Claude Proxy models (via claude-max-api-proxy)
  if (settings.value?.claude_proxy_enabled && claudeProxyModels.value.length > 0) {
    for (const m of claudeProxyModels.value) {
      models.push({ id: `claude-proxy/${m.id}`, name: `${m.name} (Proxy)`, provider: 'claude-proxy', available: claudeProxyHealthy.value })
    }
  }

  // Ollama models (dynamically fetched - always dynamic)
  for (const m of ollamaModels.value.filter(m => !m.is_embedding_model)) {
    models.push({ id: `ollama/${m.name}`, name: m.name, provider: 'ollama', available: ollamaStatus.value === 'connected' })
  }
  
  return models
})

const allEmbeddingModels = computed<ModelOption[]>(() => {
  const models: ModelOption[] = []

  // Ollama embedding models (local, free — listed first)
  for (const m of ollamaModels.value.filter(m => m.is_embedding_model)) {
    models.push({ id: `ollama/${m.name}`, name: `${m.name} (${m.size_mb}MB, local)`, provider: 'ollama', available: ollamaStatus.value === 'connected' })
  }

  // OpenAI embedding models
  if (settings.value?.openai_key_set && openaiModels.value.length > 0) {
    const embeddingModels = openaiModels.value.filter(m => m.id.startsWith('text-embedding'))
    for (const m of embeddingModels) {
      models.push({ id: `openai/${m.id}`, name: m.name, provider: 'openai', available: true })
    }
  }

  // OpenRouter embedding models (from dedicated embeddings endpoint)
  if (settings.value?.openrouter_key_set && openrouterEmbeddingModels.value.length > 0) {
    for (const m of openrouterEmbeddingModels.value) {
      models.push({ id: `openrouter/${m.id}`, name: `${m.name} (OpenRouter)`, provider: 'openrouter', available: true })
    }
  }

  // GitHub embedding models
  if (settings.value?.github_key_set && githubModels.value.length > 0) {
    const embeddingModels = githubModels.value.filter(m =>
      m.id.includes('embed') || m.id.includes('embedding')
    )
    for (const m of embeddingModels) {
      models.push({ id: `github/${m.id}`, name: m.name, provider: 'github', available: true })
    }
  }

  return models
})

// Models available for context summarization (any chat model can be used)
const allSummarizationModels = computed<ModelOption[]>(() => {
  const models: ModelOption[] = []

  // Ollama chat models (listed first - local, free, private)
  for (const m of ollamaModels.value.filter(m => !m.is_embedding_model)) {
    models.push({ id: `ollama/${m.name}`, name: `${m.name} (local)`, provider: 'ollama', available: ollamaStatus.value === 'connected' })
  }

  // Anthropic models
  const anthropicAvailable = settings.value?.anthropic_key_set || settings.value?.anthropic_oauth_logged_in
  if (anthropicAvailable && anthropicModels.value.length > 0) {
    for (const m of anthropicModels.value) {
      models.push({ id: `anthropic/${m.id}`, name: m.name, provider: 'anthropic', available: true })
    }
  }

  // OpenAI models
  if (settings.value?.openai_key_set && openaiModels.value.length > 0) {
    const chatModels = openaiModels.value.filter(m =>
      m.id.startsWith('gpt-') || m.id.startsWith('o1') || m.id.startsWith('o3') || m.id.startsWith('chatgpt')
    )
    for (const m of chatModels) {
      models.push({ id: `openai/${m.id}`, name: m.name, provider: 'openai', available: true })
    }
  }

  // OpenRouter models
  if (settings.value?.openrouter_key_set && openrouterModels.value.length > 0) {
    for (const m of openrouterModels.value) {
      models.push({ id: `openrouter/${m.id}`, name: m.name, provider: 'openrouter', available: true })
    }
  }

  // GitHub Models
  if (settings.value?.github_key_set && githubModels.value.length > 0) {
    for (const m of githubModels.value) {
      models.push({ id: `github/${m.id}`, name: m.name, provider: 'github', available: true })
    }
  }

  return models
})

// Vision-capable models for OCR
const KNOWN_VISION_MODEL_PATTERNS = [
  'llava', 'deepseek-ocr', 'minicpm-v', 'moondream', 'bakllava',
  'cogvlm', 'internvl', 'qwen-vl', 'phi-3-vision', 'phi3v',
  'gpt-4o', 'gpt-4-vision', 'gpt-4-turbo',
  'claude-3', 'claude-opus', 'claude-sonnet', 'claude-haiku',
]

function isVisionCapable(modelId: string, provider: string): boolean {
  const id = modelId.toLowerCase()
  if (provider === 'anthropic') {
    // claude-3 and above support vision
    return id.includes('claude-3') || id.includes('claude-opus') ||
           id.includes('claude-sonnet') || id.includes('claude-haiku')
  }
  if (provider === 'openai') {
    return id.includes('gpt-4o') || id.includes('gpt-4-vision') || id.includes('gpt-4-turbo')
  }
  if (provider === 'ollama') {
    return KNOWN_VISION_MODEL_PATTERNS.some(p => id.includes(p))
  }
  // openrouter / github / claude-proxy: pattern match
  return KNOWN_VISION_MODEL_PATTERNS.some(p => id.includes(p))
}

const allOcrModels = computed<ModelOption[]>(() => {
  const models: ModelOption[] = []

  // Anthropic claude-3+ (vision capable)
  const anthropicAvailable = settings.value?.anthropic_key_set || settings.value?.anthropic_oauth_logged_in
  if (anthropicAvailable && anthropicModels.value.length > 0) {
    for (const m of anthropicModels.value) {
      if (isVisionCapable(m.id, 'anthropic')) {
        models.push({ id: m.id, name: m.name, provider: 'anthropic', available: true })
      }
    }
  }

  // OpenAI vision models
  if (settings.value?.openai_key_set && openaiModels.value.length > 0) {
    for (const m of openaiModels.value) {
      if (isVisionCapable(m.id, 'openai')) {
        models.push({ id: m.id, name: m.name, provider: 'openai', available: true })
      }
    }
  }

  // Ollama vision models (local — listed after cloud for cost efficiency awareness)
  for (const m of ollamaModels.value.filter(m => !m.is_embedding_model)) {
    if (isVisionCapable(m.name, 'ollama')) {
      models.push({
        id: `ollama/${m.name}`,
        name: `${m.name} (local)`,
        provider: 'ollama',
        available: ollamaStatus.value === 'connected',
      })
    }
  }

  // OpenRouter vision models
  if (settings.value?.openrouter_key_set && openrouterModels.value.length > 0) {
    for (const m of openrouterModels.value) {
      if (isVisionCapable(m.id, 'openrouter')) {
        models.push({ id: `openrouter/${m.id}`, name: m.name, provider: 'openrouter', available: true })
      }
    }
  }

  // GitHub vision models
  if (settings.value?.github_key_set && githubModels.value.length > 0) {
    for (const m of githubModels.value) {
      if (isVisionCapable(m.id, 'github')) {
        models.push({ id: `github/${m.id}`, name: m.name, provider: 'github', available: true })
      }
    }
  }

  return models
})

const configPath = computed(() => {
  if (navigator.platform.includes('Win')) {
    return '%APPDATA%\\clawd\\Nanna\\config\\config.toml'
  } else if (navigator.platform.includes('Mac')) {
    return '~/Library/Application Support/clawd.Nanna/config.toml'
  } else {
    return '~/.config/nanna/config.toml'
  }
})

const ollamaModelOptions = computed(() => {
  return ollamaModels.value.map(m => ({
    value: m.name,
    label: `${m.name} (${m.size_mb}MB)${m.is_embedding_model ? ' ★' : ''}`
  }))
})

onMounted(async () => {
  await loadSettings()
  await loadSessions()
  await loadMemoryStats()
  await loadSimilarityThreshold()
})

async function loadSettings() {
  try {
    settings.value = await invoke<ExtendedSettings>('get_extended_settings')
    selectedModel.value = settings.value.model
    selectedEmbeddingModel.value = settings.value.embedding_model
    ollamaHostInput.value = settings.value.ollama_host
    ollamaApiKeyInput.value = settings.value.ollama_api_key || ''
    claudeProxyUrl.value = settings.value.claude_proxy_url || 'http://localhost:3456'
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
    
    // Load model priority lists
    try {
      chatModelPriority.value = await invoke<string[]>('get_chat_model_priority')
    } catch {
      // Default to current model if priority not set
      chatModelPriority.value = settings.value.model ? [settings.value.model] : []
    }
    try {
      embeddingModelPriority.value = await invoke<string[]>('get_embedding_model_priority')
    } catch {
      // Default based on current embedding config
      if (settings.value.embedding_provider !== 'disabled' && settings.value.embedding_model) {
        embeddingModelPriority.value = [`${settings.value.embedding_provider}/${settings.value.embedding_model}`]
      } else {
        embeddingModelPriority.value = []
      }
    }
    try {
      summarizationModelPriority.value = await invoke<string[]>('get_summarization_model_priority')
    } catch {
      // Default to empty (truncate instead of summarize)
      summarizationModelPriority.value = []
    }
    try {
      ocrModelPriority.value = await invoke<string[]>('get_ocr_model_priority')
    } catch {
      ocrModelPriority.value = []
    }
    try {
      useEmbeddedOcr.value = await invoke<boolean>('get_use_embedded_ocr')
    } catch {
      useEmbeddedOcr.value = true
    }

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

    // Always refresh Ollama models to populate the lists
    await refreshOllamaModels()
    await refreshModels()
  } catch (e) {
    console.error('Failed to load settings:', e)
    showToast('Failed to load settings', 'error')
  }
}

async function refreshModels() {
  if (!settings.value) return
  loadingModels.value = true
  
  // Fetch models from all available providers in parallel
  const promises: Promise<void>[] = []
  
  // Anthropic models (fetch if API key OR OAuth is available)
  if (settings.value.anthropic_key_set || settings.value.anthropic_oauth_logged_in) {
    promises.push(
      invoke<ModelInfo[]>('get_anthropic_models')
        .then(models => { anthropicModels.value = models })
        .catch(e => {
          console.warn('Failed to fetch Anthropic models:', e)
          anthropicModels.value = [] // Will use fallback in computed
        })
    )
  }
  
  // OpenAI models
  if (settings.value.openai_key_set) {
    promises.push(
      invoke<ModelInfo[]>('get_openai_models')
        .then(models => { openaiModels.value = models })
        .catch(e => {
          console.warn('Failed to fetch OpenAI models:', e)
          openaiModels.value = []
        })
    )
  }

  // OpenRouter models
  if (settings.value.openrouter_key_set) {
    promises.push(
      invoke<ModelInfo[]>('get_openrouter_models')
        .then(models => { openrouterModels.value = models })
        .catch(e => {
          console.warn('Failed to fetch OpenRouter models:', e)
          openrouterModels.value = []
        })
    )
    promises.push(
      invoke<ModelInfo[]>('get_openrouter_embedding_models')
        .then(models => { openrouterEmbeddingModels.value = models })
        .catch(e => {
          console.warn('Failed to fetch OpenRouter embedding models:', e)
          openrouterEmbeddingModels.value = []
        })
    )
  }

  // GitHub models
  if (settings.value.github_key_set) {
    promises.push(
      invoke<ModelInfo[]>('get_github_models')
        .then(models => { githubModels.value = models })
        .catch(e => {
          console.warn('Failed to fetch GitHub models:', e)
          githubModels.value = []
        })
    )
  }

  // Claude Proxy models (check health first)
  if (settings.value.claude_proxy_enabled) {
    promises.push(
      invoke<boolean>('check_claude_proxy_health')
        .then(async (healthy) => {
          claudeProxyHealthy.value = healthy
          if (healthy) {
            try {
              claudeProxyModels.value = await invoke<ModelInfo[]>('get_claude_proxy_models')
            } catch (e) {
              console.warn('Failed to fetch Claude Proxy models:', e)
              claudeProxyModels.value = []
            }
          } else {
            claudeProxyModels.value = []
          }
        })
        .catch(e => {
          console.warn('Failed to check Claude Proxy health:', e)
          claudeProxyHealthy.value = false
          claudeProxyModels.value = []
        })
    )
  }

  // Wait for all fetches to complete
  await Promise.all(promises)
  loadingModels.value = false
}

async function refreshOllamaModels() {
  loadingOllamaModels.value = true
  ollamaStatus.value = 'checking'
  ollamaError.value = ''
  try {
    ollamaModels.value = await invoke<OllamaModelInfo[]>('get_ollama_models')
    ollamaStatus.value = 'connected'
  } catch (e: any) {
    ollamaStatus.value = 'error'
    ollamaError.value = e.message || String(e)
    ollamaModels.value = []
  } finally {
    loadingOllamaModels.value = false
  }
}

async function refreshAllModels() {
  loadingModels.value = true
  try {
    // Refresh all model sources in parallel
    await Promise.all([
      refreshOllamaModels(),
      refreshModels(), // Fetches Anthropic and OpenAI
    ])
  } finally {
    loadingModels.value = false
  }
}

async function saveChatModelPriority(priority: string[]) {
  try {
    await invoke('set_chat_model_priority', { priority })
    showToast('Chat model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveEmbeddingModelPriority(priority: string[]) {
  try {
    await invoke('set_embedding_model_priority', { priority })
    showToast('Embedding model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveSummarizationModelPriority(priority: string[]) {
  try {
    await invoke('set_summarization_model_priority', { priority })
    showToast('Summarization model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveOcrModelPriority(priority: string[]) {
  try {
    await invoke('set_ocr_model_priority', { priority })
    showToast('OCR model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveUseEmbeddedOcr(enabled: boolean) {
  useEmbeddedOcr.value = enabled
  try {
    await invoke('set_use_embedded_ocr', { enabled })
    showToast(`Embedded OCR ${enabled ? 'enabled' : 'disabled'}`, 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

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

// Available models for routing (all chat models as select options)
const routingModelOptions = computed(() => {
  return allChatModels.value
    .filter(m => m.available)
    .map(m => ({
      value: m.id,
      label: `${m.name} (${m.provider})`,
    }))
})

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

async function loadSessions() {
  try {
    const sessions = await invoke<{ id: string }[]>('list_sessions')
    sessionCount.value = sessions.length
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
}

async function loadMemoryStats() {
  try {
    memoryStats.value = await invoke<CognitiveMemoryStats>('get_cognitive_memory_stats')
  } catch (e) {
    console.error('Failed to load memory stats:', e)
  }
}

async function loadSimilarityThreshold() {
  try {
    similarityThreshold.value = await invoke<number>('get_similarity_threshold')
  } catch (e) {
    console.error('Failed to load similarity threshold:', e)
  }
}

async function saveApiKey(provider: string, apiKey: string) {
  try {
    await invoke('set_provider_api_key', { provider, apiKey })
    showToast(`${formatProvider(provider)} API key saved`, 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

// =============================================================================
// Claude Proxy (claude-max-api-proxy)
// =============================================================================

async function toggleClaudeProxy(event: Event) {
  const enabled = (event.target as HTMLInputElement).checked
  try {
    await invoke('set_claude_proxy', { enabled, url: claudeProxyUrl.value })
    await loadSettings()
    if (enabled) {
      await refreshModels()
    }
    showToast(enabled ? 'Claude Proxy enabled' : 'Claude Proxy disabled', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveClaudeProxyUrl() {
  try {
    await invoke('set_claude_proxy', { enabled: true, url: claudeProxyUrl.value })
    await refreshModels()
    showToast('Claude Proxy URL saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

// =============================================================================
// Anthropic OAuth Token
// =============================================================================

async function saveOAuthToken() {
  const token = oauthTokenInput.value.trim()
  if (!token) return

  try {
    await invoke('save_anthropic_oauth_token', { token })
    oauthTokenInput.value = ''
    showToast('Anthropic token saved', 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function runClaudeSetupToken() {
  oauthLoading.value = true
  oauthAction.value = 'setup'
  try {
    const result = await invoke<string>('run_claude_setup_token')
    showToast(result || 'Successfully authenticated via Claude Code CLI', 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  } finally {
    oauthLoading.value = false
    oauthAction.value = null
  }
}

async function importClaudeCodeCredentials() {
  oauthLoading.value = true
  oauthAction.value = 'import'
  try {
    await invoke('import_claude_code_credentials')
    showToast('Successfully imported Claude Code credentials', 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed to import: ${e.message || e}`, 'error')
  } finally {
    oauthLoading.value = false
    oauthAction.value = null
  }
}

async function logoutAnthropic() {
  try {
    await invoke('logout_anthropic_oauth')
    showToast('Logged out of Anthropic', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setProvider(provider: string) {
  try {
    await invoke('set_provider', { provider })
    showToast(`Switched to ${formatProvider(provider)}`, 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function updateModel() {
  try {
    await invoke('set_model', { model: selectedModel.value })
    showToast('Model updated', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveOllamaHost() {
  try {
    await invoke('set_ollama_host', { host: ollamaHostInput.value })
    showToast('Ollama host saved', 'success')
    await refreshOllamaModels()
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveOllamaApiKey() {
  try {
    await invoke('set_ollama_api_key', { key: ollamaApiKeyInput.value })
    showToast('Ollama API key saved', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setEmbeddingProvider(provider: string) {
  try {
    if (provider === 'ollama') await refreshOllamaModels()
    let defaultModel = 'none'
    if (provider === 'openai') defaultModel = 'text-embedding-3-small'
    else if (provider === 'ollama') {
      const embeddingModel = ollamaModels.value.find(m => m.is_embedding_model)
      defaultModel = embeddingModel?.name || ollamaModels.value[0]?.name || 'nomic-embed-text'
    }
    await invoke<string>('set_embedding_config', { provider, model: defaultModel })
    showToast('Embedding config updated', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function updateEmbeddingModel() {
  try {
    const provider = settings.value?.embedding_provider || 'disabled'
    await invoke<string>('set_embedding_config', { provider, model: selectedEmbeddingModel.value })
    showToast('Embedding model updated', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setSimilarityThreshold(value: number) {
  try {
    await invoke<string>('set_similarity_threshold', { threshold: value })
    similarityThreshold.value = value
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setDreamingEnabled(enabled: boolean) {
  try {
    await invoke('set_dreaming_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setMaxCompressionRatio(value: number) {
  try {
    await invoke('set_max_compression_ratio', { ratio: value })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setMinRemainingMemories(value: number) {
  try {
    await invoke('set_min_remaining_memories', { count: value })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setSchedulerEnabled(enabled: boolean) {
  try {
    await invoke('set_scheduler_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setHeartbeatEnabled(enabled: boolean) {
  try {
    await invoke('set_heartbeat_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setHeartbeatInterval(seconds: number) {
  try {
    await invoke('set_heartbeat_interval', { seconds })
    await loadSettings()
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

async function triggerConsolidation() {
  consolidating.value = true
  try {
    const result = await invoke<{ memories_processed: number; memories_merged: number }>('trigger_consolidation')
    showToast(`Dreaming complete: ${result.memories_processed} processed`, 'success')
    await loadMemoryStats()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  } finally {
    consolidating.value = false
  }
}

async function confirmClearSessions() {
  const confirmed = await confirm({
    title: 'Delete All Sessions',
    message: 'Delete all chat sessions? This cannot be undone.',
    confirmText: 'Delete All',
    destructive: true
  })

  if (!confirmed) return

  try {
    const count = await invoke<number>('clear_all_sessions')
    showToast(`Cleared ${count} sessions`, 'success')
    sessionCount.value = 0
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function confirmClearMemories() {
  const confirmed = await confirm({
    title: 'Delete All Memories',
    message: 'Delete all memories? This cannot be undone.',
    confirmText: 'Delete All',
    destructive: true
  })

  if (!confirmed) return

  try {
    await invoke('clear_all_memories')
    showToast('All memories cleared', 'success')
    await loadMemoryStats()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveAllSettings() {
  saving.value = true
  try {
    await invoke('save_config')
    showToast('Settings saved', 'success')
    hasChanges.value = false
  } catch (e: any) {
    showToast(`Failed to save: ${e.message || e}`, 'error')
  } finally {
    saving.value = false
  }
}

async function exportConfig() {
  try {
    const config = await invoke<string>('export_config')
    
    // Create a downloadable blob
    const blob = new Blob([config], { type: 'text/plain' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'nanna-config.toml'
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
    
    showToast('Configuration exported', 'success')
  } catch (e: any) {
    showToast(`Export failed: ${e.message || e}`, 'error')
  }
}

async function importConfig() {
  try {
    // Create file input and trigger it
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.toml'
    
    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0]
      if (!file) return
      
      if (!confirm('This will replace your current configuration. Continue?')) return
      
      const content = await file.text()
      await invoke('import_config', { config: content })
      showToast('Configuration imported', 'success')
      await loadSettings()
    }
    
    input.click()
  } catch (e: any) {
    showToast(`Import failed: ${e.message || e}`, 'error')
  }
}

function formatInterval(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  return `${Math.floor(seconds / 60)} min`
}

function formatProvider(provider: string): string {
  const names: Record<string, string> = {
    anthropic: 'Anthropic', openai: 'OpenAI', openrouter: 'OpenRouter', github: 'GitHub Models', 'claude-proxy': 'Claude Proxy', ollama: 'Ollama',
  }
  return names[provider] || provider
}

function formatEmbeddingProvider(provider: string): string {
  const names: Record<string, string> = {
    openai: 'OpenAI', ollama: 'Ollama', disabled: 'Disabled',
  }
  return names[provider] || provider
}

function getToolIcon(name: string): string {
  const icons: Record<string, string> = {
    read_file: '📄', write_file: '✏️', list_dir: '📁', exec: '⚡',
    web_fetch: '🌐', web_search: '🔍', echo: '💬', analyze_image: '👁️',
    ocr: '📝', describe_image: '🖼️', read_pdf: '📑',
  }
  return icons[name] || '🔧'
}

function showToast(message: string, type: 'success' | 'error') {
  toast.value = { message, type }
  setTimeout(() => { toast.value = null }, 3000)
}
</script>

<style scoped>
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
