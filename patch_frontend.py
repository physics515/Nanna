import sys
sys.stdout.reconfigure(encoding='utf-8')

# Patch ChatInput.vue
f = 'gui/app/components/ChatInput.vue'
lines = open(f, encoding='utf-8').readlines()

changes = 0

# 1. Add ImagePlus import to lucide icons (line 183-186)
for i, line in enumerate(lines):
    if 'Send, Eye, X, Square,' in line:
        lines[i] = line.replace('Send, Eye, X, Square,', 'Send, Eye, X, Square, ImagePlus,')
        changes += 1
        break

# 2. Add ImageAttachment interface and pendingAttachments ref after line with 'const showPreview'
for i, line in enumerate(lines):
    if 'const showPreview = ref(false)' in line:
        insert = '''
interface ImageAttachment {
  id: string
  filename: string
  content_type: string
  data: string
  preview: string
}

const pendingAttachments = ref<ImageAttachment[]>([])
const MAX_IMAGE_SIZE = 5 * 1024 * 1024

function addImageFile(file: File) {
  if (file.size > MAX_IMAGE_SIZE) {
    console.warn('Image too large (max 5MB)')
    return
  }
  const reader = new FileReader()
  reader.onload = () => {
    const dataUrl = reader.result as string
    const base64 = dataUrl.split(',')[1]
    pendingAttachments.value.push({
      id: crypto.randomUUID(),
      filename: file.name,
      content_type: file.type,
      data: base64,
      preview: dataUrl,
    })
  }
  reader.readAsDataURL(file)
}

function removeAttachment(id: string) {
  pendingAttachments.value = pendingAttachments.value.filter(a => a.id !== id)
}

function openFilePicker() {
  const inp = document.createElement('input')
  inp.type = 'file'
  inp.accept = 'image/png,image/jpeg,image/gif,image/webp'
  inp.multiple = true
  inp.onchange = () => {
    if (inp.files) {
      for (const file of inp.files) {
        addImageFile(file)
      }
    }
  }
  inp.click()
}

function getAttachments() {
  const atts = pendingAttachments.value.map(a => ({
    filename: a.filename,
    content_type: a.content_type,
    data: a.data,
  }))
  pendingAttachments.value = []
  return atts
}

'''
        lines.insert(i + 1, insert)
        changes += 1
        break

# 3. Add handlePaste and handleDrop to editorProps (find handleKeyDown)
for i, line in enumerate(lines):
    if 'handleKeyDown: (view, event) =>' in line:
        paste_drop = '''    handlePaste: (view, event) => {
      const items = event.clipboardData?.items
      if (!items) return false
      for (const item of items) {
        if (item.type.startsWith('image/')) {
          event.preventDefault()
          const file = item.getAsFile()
          if (file) addImageFile(file)
          return true
        }
      }
      return false
    },
    handleDrop: (view, event) => {
      const files = event.dataTransfer?.files
      if (!files) return false
      for (const file of files) {
        if (file.type.startsWith('image/')) {
          event.preventDefault()
          addImageFile(file)
          return true
        }
      }
      return false
    },
'''
        lines.insert(i, paste_drop)
        changes += 1
        break

# 4. Update defineExpose to include getAttachments
for i, line in enumerate(lines):
    if 'defineExpose({ focus })' in line:
        lines[i] = line.replace('defineExpose({ focus })', 'defineExpose({ focus, getAttachments })')
        changes += 1
        break

# 5. Add attachment preview strip and image button to template
# Find the toolbar div (input-toolbar class) and add attachment strip before it
for i, line in enumerate(lines):
    if 'class="input-toolbar"' in line or "class='input-toolbar'" in line or 'input-toolbar' in line:
        if '<div' in lines[i-1] if i > 0 else False or '<div' in line:
            # Find the opening div for the toolbar
            j = i
            while j >= 0 and '<div' not in lines[j]:
                j -= 1
            strip_html = '''    <!-- Attachment previews -->
    <div v-if="pendingAttachments.length > 0" class="attachment-strip">
      <div v-for="att in pendingAttachments" :key="att.id" class="attachment-thumb">
        <img :src="att.preview" :alt="att.filename" />
        <button class="attachment-remove" @click="removeAttachment(att.id)">
          <X class="w-3 h-3" />
        </button>
      </div>
    </div>
'''
            lines.insert(j, strip_html)
            changes += 1
            break

# 6. Add image button to toolbar - find the toolbar buttons area
# Look for the send button or the toolbar content div
for i, line in enumerate(lines):
    if '@click="showPreview = !showPreview"' in line or 'Eye' in line:
        if 'button' in lines[i-1].lower() or 'button' in line.lower():
            # Add image button before the preview button
            j = i
            while j >= 0 and '<button' not in lines[j]:
                j -= 1
            img_btn = '''          <button
            class="p-1.5 rounded-lg transition-colors text-nanna-text-dim hover:text-nanna-text hover:bg-white/5"
            title="Attach image"
            @click="openFilePicker"
          >
            <ImagePlus class="w-4 h-4" />
          </button>
'''
            lines.insert(j, img_btn)
            changes += 1
            break

# 7. Add CSS for attachment strip
for i, line in enumerate(lines):
    if '</style>' in line:
        css = '''
/* === Attachment strip === */
.attachment-strip {
  display: flex;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
  overflow-x: auto;
}

.attachment-thumb {
  position: relative;
  flex-shrink: 0;
  width: 4rem;
  height: 4rem;
  border-radius: 0.5rem;
  overflow: hidden;
  border: 1px solid rgba(99, 102, 241, 0.3);
}

.attachment-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.attachment-remove {
  position: absolute;
  top: 0.125rem;
  right: 0.125rem;
  width: 1.25rem;
  height: 1.25rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  background: rgba(0, 0, 0, 0.7);
  color: rgba(248, 113, 113, 0.9);
  transition: all 0.15s ease;
}

.attachment-remove:hover {
  background: rgba(220, 38, 38, 0.8);
  color: white;
}

'''
        lines.insert(i, css)
        changes += 1
        break

open(f, 'w', encoding='utf-8').writelines(lines)
print(f'ChatInput.vue updated ({changes} changes)')
