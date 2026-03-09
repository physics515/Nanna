// https://nuxt.com/docs/api/configuration/nuxt-config
export default defineNuxtConfig({
  compatibilityDate: '2025-01-29',
  devtools: { enabled: true },
  
  // SSG for Tauri
  ssr: false,
  
  css: [
    '~/assets/css/fonts.css',
    '~/assets/css/main.css',
  ],

  components: [
    { path: '~/components/ui', prefix: 'Ui' },
    { path: '~/components', pathPrefix: false, ignore: ['ui/**'] },
  ],
  
  postcss: {
    plugins: {
      '@tailwindcss/postcss': {},
    },
  },
  
  app: {
    head: {
      title: 'Nanna',
      meta: [
        { name: 'description', content: 'Nanna AI Assistant' },
      ],
      link: [
        { rel: 'icon', type: 'image/png', href: '/icon.png' },
      ],
    },
  },
  
  // Tauri compatibility
  vite: {
    clearScreen: false,
    envPrefix: ['VITE_', 'TAURI_'],
    server: {
      strictPort: true,
    },
    optimizeDeps: {
      include: ['monaco-editor'],
    },
  },
})
