<script setup lang="ts">
import { ref } from 'vue'
import type { NuiRailItem } from '~/components/nui/NuiMainMenu.vue'
import type { NuiChatMenuItem } from '~/components/nui/NuiChatMenu.vue'

/**
 * New Nanna UI — chat screen assembled from the nui/ component set.
 * Faithful to the Figma design (file "Nanna", node 15:511), populated with
 * the design's sample content. Wiring to live session data replaces the
 * sample refs below; the component APIs already take real data via props.
 */
definePageMeta({ layout: false })

const navItems: NuiRailItem[] = [
  { id: 'chat', icon: 'chat', label: 'Chat' },
  { id: 'notifications', icon: 'notifications', label: 'Notifications' },
  { id: 'memory', icon: 'memory', label: 'Memory' },
  { id: 'toolbox', icon: 'toolbox', label: 'Toolbox' },
  { id: 'channels', icon: 'channels', label: 'Channels' },
  { id: 'log', icon: 'log', label: 'Log' },
  { id: 'workspaces', icon: 'workspaces', label: 'Workspaces' },
  { id: 'agents', icon: 'agents', label: 'Agents' },
  { id: 'scheduler', icon: 'scheduler', label: 'Scheduler' },
  { id: 'model-stats', icon: 'model-stats', label: 'Model statistics' },
  { id: 'tool-stats', icon: 'tool-stats', label: 'Tool statistics' },
]
const bottomNavItems: NuiRailItem[] = [
  { id: 'settings', icon: 'settings', label: 'Settings' },
]
const activeNav = ref('chat')

const chats = ref<NuiChatMenuItem[]>([
  { id: '1', title: 'Complete roadmap items P1 & P4, then make a pull request, and fix merge conflicts', meta: '35m ago' },
  { id: '2', title: 'cut a new release', meta: '1d ago' },
  { id: '3', title: 'Chat 2026-08-18 14:14', meta: '2d ago' },
  { id: '4', title: 'rename the `Task` tool to `Sub-Agent` and improve', meta: '2d ago' },
  { id: '5', title: 'use tasks to drive your subagents to complete', meta: '2d ago' },
  { id: '6', title: 'complete these roadmap items if they are not', meta: '2d ago' },
])
const activeChatId = ref('1')

const workspace = ref('default')
const workspaceOptions = [{ value: 'default', label: 'Workspace' }]

const agent = ref('default')
const agentOptions = [{ value: 'default', label: 'Default' }]

const chatTitle = ref('Complete roadmap items P1 & P4, then make a pull request, and fix merge conflicts')
const model = ref('open-router/nvidia/nemotron-3-ultra-550b-a55b:free')

const snippetFormat = ref('Markdown')

const thinkingContent = `Let me understand the situation. The user's request is as follows:
1. Complete two P0 items (publishing signed Windows .msi/.exe installer with bundled daemon sidecar, and publishing signed/notarized macOS .dmg) — signing is deferred to P0.3, so these are marked as [~] (partially complete).
2. Summarize all work done in P0 so it takes up less space on the roadmap.
3. Create a PR.

The step was interrupted mid-execution due to a provider failure. Tool calls from the interrupted attempt may have already been executed. I need to re-read the working artifacts before writing the entire file.

Let me start by discovering tools and checking the state of the repository — git status, the roadmap file, etc.`

const toolOutput = `Showing tools 1-6 of 9 total:

## exec
Execute a shell command in a POSIX bash shell (Git Bash on Windows, sh on Unix) and return its output. ALWAYS bash syntax: pipes, &&, ||, [ -f x ] / [ -d x ], ls, cat/grep/tail, mkdir -p, 2>/dev/null, forward-slash paths. NEVER cmd.exe syntax — 'if exist', '2>nul', 'cd /d', 'errorlevel' all FAIL here. To search code, use the code_search tool — rg/ripgrep is not guaranteed on PATH. Use for build commands, scripts, git operations, etc. After a command that redirects into a script or JSON file, the cheapest structural check runs on that file and its verdict is appended to the output.
Params: command (string, required), timeout (integer), workdir (string)

## status
Get system status information. Shows platform, working directory, git status, and environment overview.
Params: verbose (boolean)

## edit_file
Replace one exact text snippet in a file with new text — an in-place edit for small changes. Use this instead of rewriting the whole file with write_file. ALL THREE main parameters are REQUIRED: file_path, old_string, new_string. old_string must be text that exists in the file (copy it verbatim; indentation differences are tolerated) — include 2-3 surrounding lines to make it unique. Only the matched snippet changes; the rest of the file is untouched. After each edit the cheapest structural check (sh -n / node --check / JSON.parse) runs on the result and its verdict is appended — including whether the file parsed before the edit. Use write_file only for new files or full rewrites.
Params: file_path (string, required), new_string (string, required), occurrence (integer), old_string (string, required), replace_all (boolean)

## project_structure
Show the directory tree of a project: names, nesting and file sizes. Reads no file contents, so it reports no line counts - use code_search or read_file for what is inside a file. Noise dirs (node_modules, .git, target, ...) are shown but never descended into.
Params: max_depth (integer), path (string)

## read_file
Read a file from the filesystem. Returns the file contents with line numbers. Supports optional offset and limit for reading portions of large files.
Params: file_path (string, required), limit (integer), offset (integer)

## python
Execute Python code or manage saved scripts. Embedded interpreter — no system Python required. Supports standard library (os, json, re, pathlib, collections, math, etc). No pip/third-party packages. Use for file manipulation, data processing, batch edits, text transforms, and scripting.
Params: action (string), args (string), code (string), name (string), timeout (integer), workdir (string)

The 6 tool(s) above are ACTIVATED and ready to use for the rest of this run — call them directly, you do not need to discover them again.

*** THIS IS A PARTIAL LIST — 3 more tool(s) matched but are NOT shown and NOT yet activated. *** Nothing failed and nothing is missing from the system; results come a page at a time. If what you need is not above, get the next page with:
    discover_tools(query="run shell commands git status and read edit files", offset=6)
Results are ranked best-match first, so the next page is a worse match than this one — before paging on, consider whether a tool above already does the job.`

interface DemoTask {
  id: string
  title: string
  status: 'upcoming' | 'in-progress' | 'complete'
  description: string
  progress?: number
}

const tasks = ref<DemoTask[]>([
  { id: 't1', title: 'Verify', status: 'upcoming', description: 'Verify the new create_tool and edit_tool skills load cleanly in the tool registry.', progress: 0.02 },
  { id: 't2', title: 'Verify', status: 'in-progress', description: 'Verify the new create_tool and edit_tool skills load cleanly in the tool registry.', progress: 0.375 },
  { id: 't3', title: 'Verify', status: 'complete', description: 'Verify the new create_tool and edit_tool skills load cleanly in the tool registry.', progress: 0.375 },
])

const draft = ref('')
/** Plain-text user messages appended by the demo send action. */
const sentMessages = ref<string[]>([])

function send() {
  const text = draft.value.trim()
  if (!text) return
  sentMessages.value.push(text)
  draft.value = ''
}

async function windowControl(action: 'minimize' | 'maximize' | 'close') {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const win = getCurrentWindow()
    if (action === 'minimize') await win.minimize()
    else if (action === 'maximize') await win.toggleMaximize()
    else await win.close()
  } catch (e) {
    console.warn('Window control unavailable outside Tauri:', e)
  }
}
</script>

<template>
  <div class="nui-root flex h-screen w-screen flex-col gap-4 overflow-clip rounded-[32px] text-xs leading-normal">
    <div class="flex min-h-0 w-full flex-1 items-start gap-4">
      <NuiMainMenu
        :items="navItems"
        :bottom-items="bottomNavItems"
        :active-id="activeNav"
        class="self-stretch"
        @select="activeNav = $event"
      />

      <NuiChatMenu
        :chats="chats"
        :active-id="activeChatId"
        class="self-stretch"
        @select="activeChatId = $event"
        @new-chat="chats.unshift({ id: String(Date.now()), title: 'New chat', meta: 'now' })"
      />

      <!-- ═══ Body column ═══ -->
      <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-4 self-stretch">
        <!-- Top bar: workspace select + window controls -->
        <div class="flex w-full items-start gap-4" data-tauri-drag-region>
          <NuiSelect
            v-model="workspace"
            :options="workspaceOptions"
            icon="workspaces"
            variant="attached"
            class="w-64 shrink-0"
          />
          <div class="min-w-0 flex-1" data-tauri-drag-region />
          <NuiWindowControls
            @minimize="windowControl('minimize')"
            @maximize="windowControl('maximize')"
            @close="windowControl('close')"
          />
        </div>

        <NuiChatHeader :title="chatTitle">
          <NuiSelect v-model="agent" :options="agentOptions" class="w-64 shrink-0" />
          <NuiTag :label="model" color="info" class="shrink-0" />
        </NuiChatHeader>

        <!-- Content row: messages + action pane + action rail -->
        <div class="flex min-h-0 w-full flex-1 items-start gap-4">
          <!-- Messages -->
          <div class="nui-scroll flex min-h-0 min-w-0 flex-1 flex-col gap-8 self-stretch overflow-y-auto">
            <NuiMessage role="user">
              <div class="flex w-full flex-col items-start gap-1">
                <p class="w-full break-words text-xs leading-normal text-nui-fg">complete these two items:</p>
                <NuiCodeBlock v-model:format="snippetFormat" :formats="['Markdown', 'Plain text']" copyable>
                  <pre class="whitespace-pre-wrap break-words font-nui text-xs font-[450] leading-normal text-nui-fg"><span class="text-nui-pink">##</span> <span class="text-nui-green">P0 - Public Preview Release

</span><span class="text-nui-pink">[~]</span><span class="text-nui-green"> Publish signed Windows .msi/.exe installer with bundled daemon sidecar. (signing deferred to P0.3)
</span><span class="text-nui-pink">[~]</span><span class="text-nui-green"> Publish signed and notarized macOS .dmg (Universal or separate Intel/Apple Silicon). (notarization deferred to P0.3)</span></pre>
                </NuiCodeBlock>
                <p class="w-full break-words text-xs leading-normal text-nui-fg">then summarize all work done in P0 so it takes up less room in the roadmap. then make a pr.</p>
              </div>
            </NuiMessage>

            <NuiThinkingMessage :content="thinkingContent" default-expanded />

            <NuiMessage role="assistant">
              <div class="flex w-full flex-col items-start">
                <p class="w-full break-words text-xs leading-normal text-nui-fg">The step was interrupted mid-flight, so first I'll check what the previous attempt already changed on disk (git status + the roadmap file) before doing anything else.</p>
              </div>
            </NuiMessage>

            <NuiToolCallMessage
              :tool-call="{
                id: 'demo-1',
                name: 'discover_tools',
                input: { limit: 8, query: 'run shell commands, read and edit files, search code' },
                output: toolOutput,
                success: true,
                duration_ms: 412,
              }"
              status="completed"
              default-expanded
            />

            <NuiMessage v-for="(msg, i) in sentMessages" :key="i" role="user">
              <p class="w-full whitespace-pre-wrap break-words text-xs leading-normal text-nui-fg">{{ msg }}</p>
            </NuiMessage>
          </div>

          <!-- Action pane: tasks -->
          <div class="nui-scroll flex w-[512px] shrink-0 flex-col gap-8 self-stretch overflow-y-auto">
            <NuiTaskCard
              v-for="task in tasks"
              :key="task.id"
              :title="task.title"
              :status="task.status"
              :description="task.description"
              :progress="task.progress"
              @complete="task.status = 'complete'"
              @delete="tasks = tasks.filter(t => t.id !== task.id)"
            />
          </div>

          <!-- Action rail -->
          <div class="flex shrink-0 flex-col items-start">
            <NuiRailButton icon="tasks" label="Tasks" active accent="green" />
          </div>
        </div>

        <!-- Chatbox -->
        <div class="flex w-full shrink-0 items-start px-32 py-2">
          <NuiChatbox v-model="draft" @send="send" />
        </div>
      </div>
    </div>

    <NuiStatusBar
      ui-version="3.10"
      server-version="3.10"
      connected
      status-text="Connected to server"
    />
  </div>
</template>
