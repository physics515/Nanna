<template>
  <UiModal v-if="showCloseDialog" v-model="showCloseDialog" title="Close Nanna?" size="sm" :closable="true">
    <div class="space-y-4">
      <p class="text-nanna-text-muted">
        What would you like to do?
      </p>
      
      <div class="space-y-2">
        <button
          @click="minimizeToTray"
          class="w-full flex items-center gap-3 p-3 rounded-lg border border-nanna-border hover:border-nanna-accent hover:bg-nanna-accent/10 transition-colors text-left"
        >
          <div class="text-2xl">🔽</div>
          <div>
            <div class="font-medium text-nanna-text">Minimize to Tray</div>
            <div class="text-sm text-nanna-text-muted">
              Hide the window but keep Nanna running in the background
            </div>
          </div>
        </button>
        
        <button
          @click="quitCompletely"
          class="w-full flex items-center gap-3 p-3 rounded-lg border border-nanna-border hover:border-red-500 hover:bg-red-500/10 transition-colors text-left"
        >
          <div class="text-2xl">🚪</div>
          <div>
            <div class="font-medium text-nanna-text">Quit Completely</div>
            <div class="text-sm text-nanna-text-muted">
              Close Nanna and stop all background services
            </div>
          </div>
        </button>
      </div>
      
      <label class="flex items-center gap-2 text-sm text-nanna-text-muted cursor-pointer">
        <input 
          type="checkbox" 
          v-model="rememberChoice"
          class="rounded border-nanna-border text-nanna-accent focus:ring-nanna-accent"
        />
        <span>Remember my choice</span>
      </label>
    </div>
    
    <template #footer>
      <div class="flex justify-end">
        <UiButton variant="ghost" @click="cancelClose">
          Cancel
        </UiButton>
      </div>
    </template>
  </UiModal>
</template>

<script setup lang="ts">
const { 
  showCloseDialog, 
  rememberChoice,
  minimizeToTray, 
  quitCompletely, 
  cancelClose 
} = useCloseHandler()
</script>
