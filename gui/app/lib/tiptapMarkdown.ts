/**
 * Markdown ↔ Tiptap conversion used by RichTextEditor.
 *
 * Extracted from the component so the outbound path (Tiptap JSON → markdown)
 * can be unit-tested directly: this is the function whose output becomes the
 * message the daemon receives, so a corruption here is a corruption of what
 * the user actually said.
 */

export function markdownToHtml(md: string): string {
  if (!md) return ''

  const lines = md.split('\n')
  const html: string[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]

    // Fenced code blocks → MonacoCodeBlock node
    if (line.startsWith('```')) {
      const lang = line.slice(3).trim()
      const codeLines: string[] = []
      i++
      while (i < lines.length && !lines[i].startsWith('```')) {
        codeLines.push(lines[i])
        i++
      }
      i++ // skip closing ```
      const content = codeLines.join('\n')
      // MonacoCodeBlock is an atom node — set attrs via data attributes
      html.push(`<monaco-code-block language="${escAttr(lang)}" content="${escAttr(content)}"></monaco-code-block>`)
      continue
    }

    // Headings
    const headingMatch = line.match(/^(#{1,6})\s+(.*)/)
    if (headingMatch) {
      const level = headingMatch[1].length
      html.push(`<h${level}>${inlineMd(headingMatch[2])}</h${level}>`)
      i++
      continue
    }

    // Blockquote
    if (line.startsWith('> ')) {
      const quoteLines: string[] = []
      while (i < lines.length && lines[i].startsWith('> ')) {
        quoteLines.push(lines[i].slice(2))
        i++
      }
      html.push(`<blockquote><p>${inlineMd(quoteLines.join('<br>'))}</p></blockquote>`)
      continue
    }

    // Task list
    if (line.match(/^- \[([ x])\]\s/)) {
      const items: string[] = []
      while (i < lines.length) {
        const tm = lines[i].match(/^- \[([ x])\]\s+(.*)/)
        if (!tm) break
        const checked = tm[1] === 'x' ? ' data-checked="true"' : ''
        items.push(`<li data-type="taskItem"${checked}><p>${inlineMd(tm[2])}</p></li>`)
        i++
      }
      html.push(`<ul data-type="taskList">${items.join('')}</ul>`)
      continue
    }

    // Unordered list
    if (line.match(/^[-*]\s+/)) {
      const items: string[] = []
      while (i < lines.length && lines[i].match(/^[-*]\s+/)) {
        items.push(`<li><p>${inlineMd(lines[i].replace(/^[-*]\s+/, ''))}</p></li>`)
        i++
      }
      html.push(`<ul>${items.join('')}</ul>`)
      continue
    }

    // Ordered list
    if (line.match(/^\d+\.\s+/)) {
      const items: string[] = []
      while (i < lines.length && lines[i].match(/^\d+\.\s+/)) {
        items.push(`<li><p>${inlineMd(lines[i].replace(/^\d+\.\s+/, ''))}</p></li>`)
        i++
      }
      html.push(`<ol>${items.join('')}</ol>`)
      continue
    }

    // HR
    if (line.match(/^---+$/)) {
      html.push('<hr>')
      i++
      continue
    }

    // Image
    const imgMatch = line.match(/^!\[([^\]]*)\]\(([^)]+)\)$/)
    if (imgMatch) {
      html.push(`<img src="${escAttr(imgMatch[2])}" alt="${escAttr(imgMatch[1])}" />`)
      i++
      continue
    }

    // Empty line
    if (!line.trim()) { i++; continue }

    // Paragraph
    html.push(`<p>${inlineMd(line)}</p>`)
    i++
  }

  return html.join('')
}

export function inlineMd(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    .replace(/~~(.+?)~~/g, '<s>$1</s>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>')
}

export function escAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

// ── Tiptap JSON → Markdown ──

export function jsonToMarkdown(doc: any): string {
  if (!doc?.content) return ''

  return doc.content.map((node: any) => {
    switch (node.type) {
      case 'paragraph':
        return nodeContentToText(node)
      case 'heading': {
        const level = node.attrs?.level || 1
        return '#'.repeat(level) + ' ' + nodeContentToText(node)
      }
      case 'monacoCodeBlock': {
        const lang = node.attrs?.language || ''
        const code = node.content?.[0]?.text || node.attrs?.content || ''
        return '```' + lang + '\n' + code + '\n```'
      }
      case 'bulletList':
        return node.content?.map((item: any) => '- ' + nodeContentToText(item.content?.[0])).join('\n') || ''
      case 'orderedList':
        return node.content?.map((item: any, i: number) => `${i + 1}. ` + nodeContentToText(item.content?.[0])).join('\n') || ''
      case 'taskList':
        return node.content?.map((item: any) => {
          const checked = item.attrs?.checked ? 'x' : ' '
          return `- [${checked}] ` + nodeContentToText(item.content?.[0])
        }).join('\n') || ''
      case 'blockquote':
        return node.content?.map((p: any) => '> ' + nodeContentToText(p)).join('\n') || ''
      case 'horizontalRule':
        return '---'
      case 'image':
        return `![${node.attrs?.alt || ''}](${node.attrs?.src || ''})`
      default:
        return nodeContentToText(node)
    }
  }).reduce((acc: string, block: string, i: number, arr: string[]) => {
    if (i === 0) return block
    const prev = arr[i - 1]
    const isCodeBlock = block.startsWith('```')
    const prevIsCodeBlock = prev.endsWith('```')
    if (isCodeBlock || prevIsCodeBlock) return acc + '\n' + block
    return acc + '\n\n' + block
  }, '').trim()
}

export function nodeContentToText(node: any): string {
  if (!node?.content) return ''
  return node.content.map((item: any) => {
    if (item.type === 'image') {
      return `![${item.attrs?.alt || ''}](${item.attrs?.src || ''})`
    }
    let text = item.text || ''
    if (item.marks) {
      for (const mark of item.marks) {
        switch (mark.type) {
          case 'bold': text = `**${text}**`; break
          case 'italic': text = `*${text}*`; break
          case 'strike': text = `~~${text}~~`; break
          case 'code': text = '`' + text + '`'; break
          case 'link': text = `[${text}](${mark.attrs?.href || ''})`; break
        }
      }
    }
    return text
  }).join('')
}
