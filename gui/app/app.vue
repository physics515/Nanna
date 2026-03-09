<template>
  <NuxtLayout>
    <NuxtPage />
  </NuxtLayout>
</template>

<style>
html, body {
  margin: 0;
  padding: 0;
  overflow: hidden;
  height: 100%;
  background-color: #1e293b;
  color: #e2e8f0;
  font-family: 'clother', 'Inter', system-ui, sans-serif;
}

/* Glass scrollbar */
::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
::-webkit-scrollbar-track {
  background: transparent;
}
::-webkit-scrollbar-thumb {
  background: rgba(139, 92, 246, 0.18);
  border-radius: 3px;
  border: 1px solid rgba(255, 255, 255, 0.06);
}
::-webkit-scrollbar-thumb:hover {
  background: rgba(139, 92, 246, 0.35);
}
::-webkit-scrollbar-corner {
  background: transparent;
}
</style>
