// eslint.config.js - ESLint configuration for Nanna GUI
import vue from 'eslint-plugin-vue'
import pluginVue from 'eslint-plugin-vue'
import parserVue from '@vue/eslint-parser'

export default [
  {
    name: 'app/files/javascript',
    files: ['**/*.js'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
    },
    plugins: {
      vue,
    },
    rules: {
      'no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
    },
  },
  {
    name: 'app/files/typescript',
    files: ['**/*.ts'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
    },
    plugins: {
      vue,
    },
    rules: {
      'no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
    },
  },
  {
    name: 'app/files/vue',
    files: ['**/*.vue'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      parser: parserVue,
    },
    plugins: {
      vue: pluginVue,
    },
    rules: {
      'vue/multi-word-component-names': 'off',
      'vue/no-template-shadow': 'warn',
      'vue/require-default-prop': 'warn',
      'vue/define-macros-order': 'error',
      'vue/no-v-html': 'warn',
    },
  },
]
