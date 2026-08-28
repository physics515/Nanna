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

  experimental: {
    // A route chunk that fails to load must never leave a blank shell. The Nuxt
    // default ('automatic') defers the recovery reload until the *next*
    // navigation — but the app is already blank, so there is nothing left to
    // navigate with and the window sits dead until the user hits Ctrl+R.
    // 'automatic-immediate' reloads on the spot.
    emitRouteChunkError: 'automatic-immediate',
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
      // Every page is a lazily-imported route chunk, so Vite's dep scanner only
      // meets a dependency the first time its page is opened. Discovering one
      // mid-session triggers a re-optimize that 504s the in-flight chunk
      // ("Outdated Optimize Dep") — the route never mounts and the shell goes
      // blank until a manual reload. `@tauri-apps/plugin-dialog` (imported only
      // by pages/workspaces.vue) did exactly that to the Workspaces page.
      //
      // Rule: every bare specifier imported anywhere under app/ belongs here, so
      // the whole set is bundled once at server start and never again.
      // Regenerate with:
      //   grep -rhoE "from '[^.~#'][^']*'" app/ | sed "s/from '//;s/'//" | sort -u
      include: [
        '@guolao/vue-monaco-editor',
        '@tauri-apps/api/app',
        '@tauri-apps/api/core',
        '@tauri-apps/api/event',
        '@tauri-apps/api/window',
        '@tauri-apps/plugin-dialog',
        '@tauri-apps/plugin-notification',
        '@tauri-apps/plugin-process',
        '@tauri-apps/plugin-updater',
        // '@tiptap/core' is imported by app/extensions/* but is not a declared
        // dependency, so it does not resolve from the project root. It is
        // already folded into the starter-kit / vue-3 bundles below.
        '@tiptap/extension-image',
        '@tiptap/extension-link',
        '@tiptap/extension-placeholder',
        '@tiptap/extension-task-item',
        '@tiptap/extension-task-list',
        '@tiptap/extension-typography',
        '@tiptap/starter-kit',
        '@tiptap/suggestion',
        '@tiptap/vue-3',
        'class-variance-authority',
        'clsx',
        '@lucide/vue',
        'marked',
        'monaco-editor',
        'radix-vue',
        'tailwind-merge',
        'tippy.js',
        'vue-sonner',
        'vuedraggable',
      ],
    },
  },
})
