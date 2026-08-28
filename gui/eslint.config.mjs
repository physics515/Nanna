/** @type {import('eslint').Linter.Config} */
export default tseslint.config({
  ignores: ['dist', '.nuxt', '.output'],
  plugins: {
    '@typescript-eslint': tseslint.plugin('@typescript-eslint/eslint-plugin'),
  },
  languageOptions: {
    ecmaVersion: 'latest',
    sourceType: 'module',
    parser: tseslint.parser(),
    parserOptions: {
      project: true,
      tsconfigRootDir: import.meta.dirname,
    },
  },
  linterRules: {
    '@typescript-eslint/no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
    '@typescript-eslint/no-explicit-any': 'warn',
  },
});
