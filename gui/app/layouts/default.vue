<template>
  <div class="min-h-screen bg-nanna-bg-deep bg-grid relative">
    <!-- Scanlines overlay (optional) -->
    <!-- <div class="scanlines absolute inset-0 pointer-events-none z-50" /> -->
    
    <!-- Main content -->
    <div class="flex h-screen">
      <!-- Sidebar -->
      <aside class="w-64 bg-nanna-bg-surface border-r border-nanna-primary/10 flex flex-col">
        <!-- Logo -->
        <div class="p-4 border-b border-nanna-primary/10">
          <h1 class="text-2xl font-bold text-nanna-accent crt-glow">
            NANNA
          </h1>
          <p class="text-xs text-nanna-text-muted mt-1">
            AI Assistant
          </p>
        </div>
        
        <!-- Sessions list -->
        <nav class="flex-1 p-4 space-y-2 overflow-y-auto">
          <button class="w-full btn-primary text-left">
            + New Chat
          </button>
          
          <div class="mt-4 space-y-1">
            <div class="text-xs text-nanna-text-dim uppercase tracking-wider mb-2">
              Recent Chats
            </div>
            <!-- Session items will go here -->
            <button class="w-full text-left px-3 py-2 rounded-lg text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
              Session 1
            </button>
          </div>
        </nav>
        
        <!-- Footer -->
        <div class="p-4 border-t border-nanna-primary/10">
          <div class="text-xs text-nanna-text-dim">
            v0.1.0 • <span class="text-nanna-success">●</span> Connected
          </div>
        </div>
      </aside>
      
      <!-- Main area -->
      <main class="flex-1 flex flex-col">
        <slot />
      </main>
    </div>
  </div>
</template>
