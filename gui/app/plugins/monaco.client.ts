// Monaco editor setup - client only
import { loader } from '@guolao/vue-monaco-editor'
import * as monaco from 'monaco-editor'
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker'
import jsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker'
import cssWorker from 'monaco-editor/esm/vs/language/css/css.worker?worker'
import htmlWorker from 'monaco-editor/esm/vs/language/html/html.worker?worker'
import tsWorker from 'monaco-editor/esm/vs/language/typescript/ts.worker?worker'

export default defineNuxtPlugin(() => {
  // Set up Monaco environment for workers
  self.MonacoEnvironment = {
    getWorker(_: unknown, label: string) {
      if (label === 'json') {
        return new jsonWorker()
      }
      if (label === 'css' || label === 'scss' || label === 'less') {
        return new cssWorker()
      }
      if (label === 'html' || label === 'handlebars' || label === 'razor') {
        return new htmlWorker()
      }
      if (label === 'typescript' || label === 'javascript') {
        return new tsWorker()
      }
      return new editorWorker()
    },
  }

  // Configure the loader to use our local monaco instance
  loader.config({ monaco })

  // Define the custom theme
  monaco.editor.defineTheme('nanna-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      // General
      { token: 'comment', foreground: '64748b', fontStyle: 'italic' },
      { token: 'keyword', foreground: 'c792ea' },
      { token: 'keyword.control', foreground: 'c792ea' },
      { token: 'keyword.operator', foreground: '89ddff' },
      { token: 'string', foreground: 'c3e88d' },
      { token: 'string.escape', foreground: '89ddff' },
      { token: 'number', foreground: 'f78c6c' },
      { token: 'number.float', foreground: 'f78c6c' },
      { token: 'number.hex', foreground: 'f78c6c' },
      { token: 'type', foreground: 'ffcb6b' },
      { token: 'type.identifier', foreground: 'ffcb6b' },
      { token: 'function', foreground: '82aaff' },
      { token: 'function.declaration', foreground: '82aaff' },
      { token: 'variable', foreground: 'e2e8f0' },
      { token: 'variable.predefined', foreground: '89ddff' },
      { token: 'constant', foreground: '89ddff' },
      { token: 'operator', foreground: '89ddff' },
      { token: 'delimiter', foreground: 'e2e8f0' },
      { token: 'delimiter.bracket', foreground: 'e2e8f0' },
      
      // Rust specific
      { token: 'keyword.rust', foreground: 'c792ea' },
      { token: 'lifetime.rust', foreground: 'f78c6c' },
      { token: 'attribute.rust', foreground: 'ffcb6b' },
      { token: 'macro.rust', foreground: '82aaff' },
      
      // Python specific
      { token: 'keyword.python', foreground: 'c792ea' },
      { token: 'decorator.python', foreground: 'ffcb6b' },
      { token: 'builtin.python', foreground: '89ddff' },
      
      // C/C++ specific
      { token: 'keyword.cpp', foreground: 'c792ea' },
      { token: 'keyword.c', foreground: 'c792ea' },
      { token: 'preprocessor', foreground: 'c792ea' },
      { token: 'preprocessor.cpp', foreground: 'c792ea' },
      { token: 'type.cpp', foreground: 'ffcb6b' },
      { token: 'namespace.cpp', foreground: 'ffcb6b' },
      
      // Annotations/Attributes
      { token: 'annotation', foreground: 'ffcb6b' },
      { token: 'metatag', foreground: 'ffcb6b' },
      { token: 'tag', foreground: 'f07178' },
      { token: 'attribute.name', foreground: 'ffcb6b' },
      { token: 'attribute.value', foreground: 'c3e88d' },
    ],
    colors: {
      'editor.background': '#1e293b',
      'editorGutter.background': '#1e293b',
      'editor.foreground': '#e2e8f0',
      'editor.lineHighlightBackground': '#334155',
      'editor.selectionBackground': '#6d28d980',
      'editorLineNumber.foreground': '#64748b',
      'editorLineNumber.activeForeground': '#94a3b8',
      'editorCursor.foreground': '#22d3ee',
      'editor.selectionHighlightBackground': '#6d28d940',
      'editorBracketMatch.background': '#6d28d940',
      'editorBracketMatch.border': '#8b5cf6',
      'scrollbarSlider.background': '#8b5cf640',
      'scrollbarSlider.hoverBackground': '#8b5cf680',
      'scrollbarSlider.activeBackground': '#8b5cf6',
    },
  })

  // Transparent variant — same syntax colours, no solid backgrounds.
  // Used by TiptapMonacoBlock so the splatter gradient shows through.
  monaco.editor.defineTheme('nanna-dark-transparent', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: 'comment', foreground: '64748b', fontStyle: 'italic' },
      { token: 'keyword', foreground: 'c792ea' },
      { token: 'keyword.control', foreground: 'c792ea' },
      { token: 'keyword.operator', foreground: '89ddff' },
      { token: 'string', foreground: 'c3e88d' },
      { token: 'string.escape', foreground: '89ddff' },
      { token: 'number', foreground: 'f78c6c' },
      { token: 'number.float', foreground: 'f78c6c' },
      { token: 'number.hex', foreground: 'f78c6c' },
      { token: 'type', foreground: 'ffcb6b' },
      { token: 'type.identifier', foreground: 'ffcb6b' },
      { token: 'function', foreground: '82aaff' },
      { token: 'function.declaration', foreground: '82aaff' },
      { token: 'variable', foreground: 'e2e8f0' },
      { token: 'variable.predefined', foreground: '89ddff' },
      { token: 'constant', foreground: '89ddff' },
      { token: 'operator', foreground: '89ddff' },
      { token: 'delimiter', foreground: 'e2e8f0' },
      { token: 'delimiter.bracket', foreground: 'e2e8f0' },
      { token: 'keyword.rust', foreground: 'c792ea' },
      { token: 'lifetime.rust', foreground: 'f78c6c' },
      { token: 'attribute.rust', foreground: 'ffcb6b' },
      { token: 'macro.rust', foreground: '82aaff' },
      { token: 'keyword.python', foreground: 'c792ea' },
      { token: 'decorator.python', foreground: 'ffcb6b' },
      { token: 'builtin.python', foreground: '89ddff' },
      { token: 'keyword.cpp', foreground: 'c792ea' },
      { token: 'keyword.c', foreground: 'c792ea' },
      { token: 'preprocessor', foreground: 'c792ea' },
      { token: 'preprocessor.cpp', foreground: 'c792ea' },
      { token: 'type.cpp', foreground: 'ffcb6b' },
      { token: 'namespace.cpp', foreground: 'ffcb6b' },
      { token: 'annotation', foreground: 'ffcb6b' },
      { token: 'metatag', foreground: 'ffcb6b' },
      { token: 'tag', foreground: 'f07178' },
      { token: 'attribute.name', foreground: 'ffcb6b' },
      { token: 'attribute.value', foreground: 'c3e88d' },
    ],
    colors: {
      'editor.background': '#00000000',
      'editorGutter.background': '#00000000',
      'editor.foreground': '#e2e8f0',
      'editor.lineHighlightBackground': '#33415580',
      'editor.selectionBackground': '#6d28d980',
      'editorLineNumber.foreground': '#64748b',
      'editorLineNumber.activeForeground': '#94a3b8',
      'editorCursor.foreground': '#22d3ee',
      'editor.selectionHighlightBackground': '#6d28d940',
      'editorBracketMatch.background': '#6d28d940',
      'editorBracketMatch.border': '#8b5cf6',
      'scrollbarSlider.background': '#8b5cf640',
      'scrollbarSlider.hoverBackground': '#8b5cf680',
      'scrollbarSlider.activeBackground': '#8b5cf6',
    },
  })

  return {
    provide: {
      monaco,
    },
  }
})
