// https://nuxt.com/docs/api/configuration/nuxt-config
export default defineNuxtConfig({
  compatibilityDate: '2025-01-29',

  // Keep the build tree project-local. On Windows, Nuxt 4 can otherwise land
  // artifacts under node_modules/.cache/nuxt/.nuxt — a concurrent client / nitro
  // pass then races on dist/client/manifest.json (ENOENT mid-generate).
  buildDir: '.nuxt',

  devtools: { enabled: true },
  $production: {
    devtools: { enabled: false },
  },

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
      htmlAttrs: {
        lang: 'en',
      },
      title: 'Nanna',
      meta: [
        { name: 'description', content: 'Nanna AI Assistant' },
      ],
      link: [
        { rel: 'icon', type: 'image/png', href: '/icon.png' },
      ],
    },
  },

  nitro: {
    preset: 'static',
    // SPA shell only — no SSR HTML bodies. Avoids the client-manifest read
    // path that races potently when prerenderer and Vite finish out of order.
    prerender: {
      crawlLinks: false,
      routes: ['/'],
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
