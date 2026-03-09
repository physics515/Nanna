import type { Config } from 'tailwindcss'

export default {
  content: [
    './app/**/*.{vue,js,ts,jsx,tsx}',
    './components/**/*.{vue,js,ts,jsx,tsx}',
    './layouts/**/*.vue',
    './pages/**/*.vue',
    './plugins/**/*.{js,ts}',
    './nuxt.config.{js,ts}',
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        // Palenight-inspired — adapted from nisaba design system
        nanna: {
          bg: {
            DEFAULT: '#0f172a', // deep sidebar / main bg
            deep: '#020617',    // darkest
            surface: '#1e293b', // card surfaces
            elevated: '#334155', // elevated elements
          },
          primary: {
            DEFAULT: '#8b5cf6', // violet-500 accent
            hover: '#a78bfa',   // violet-400
            muted: '#6d28d9',   // violet-700
          },
          secondary: {
            DEFAULT: '#818cf8', // indigo-400
            hover: '#a5b4fc',   // indigo-300
          },
          accent: {
            DEFAULT: '#22d3ee', // cyan-400
            hover: '#67e8f9',   // cyan-300
            glow: '#06b6d4',    // cyan-500
          },
          text: {
            DEFAULT: '#e2e8f0', // foreground
            muted: '#c4cdd6',   // muted text (nisaba muted)
            dim: '#64748b',     // slate-500
          },
          border: '#475569',    // nisaba border
          success: '#34d399',   // emerald-400
          warning: '#fbbf24',   // amber-400
          error: '#fb7185',     // rose-400
        },
      },
      fontFamily: {
        sans: ['clother', 'Inter', 'system-ui', 'sans-serif'],
        heading: ['pulpo-rust-50', 'serif'],
        glass: ['jubilat', 'serif'],
        mono: ['JetBrains Mono', 'Fira Code', 'monospace'],
      },
      boxShadow: {
        'glow': '0 0 20px rgba(139, 92, 246, 0.3)',
        'glow-sm': '0 0 10px rgba(139, 92, 246, 0.2)',
        'glow-violet': '0 0 20px rgba(139, 92, 246, 0.3)',
        'glow-cyan': '0 0 20px rgba(34, 211, 238, 0.3)',
        'glass': 'inset 0 1px 0 0 rgba(255,255,255,0.04), 0 1.5px 1px -0.5px rgba(0,0,0,0.18), 0 3px 8px -3px rgba(0,0,0,0.12)',
      },
      animation: {
        'pulse-glow': 'pulse-glow 2s ease-in-out infinite',
        'scan': 'scan 8s linear infinite',
        'blink': 'blink 1s step-end infinite',
      },
      keyframes: {
        'pulse-glow': {
          '0%, 100%': { opacity: '1' },
          '50%': { opacity: '0.5' },
        },
        'scan': {
          '0%': { transform: 'translateY(-100%)' },
          '100%': { transform: 'translateY(100%)' },
        },
        'blink': {
          '0%, 100%': { opacity: '1' },
          '50%': { opacity: '0' },
        },
      },
      backgroundImage: {
        'grid-pattern': `linear-gradient(rgba(139, 92, 246, 0.03) 1px, transparent 1px),
                         linear-gradient(90deg, rgba(139, 92, 246, 0.03) 1px, transparent 1px)`,
        'scanlines': `repeating-linear-gradient(
          0deg,
          transparent,
          transparent 2px,
          rgba(0, 0, 0, 0.1) 2px,
          rgba(0, 0, 0, 0.1) 4px
        )`,
      },
      backgroundSize: {
        'grid': '20px 20px',
      },
    },
  },
  plugins: [],
} satisfies Config
