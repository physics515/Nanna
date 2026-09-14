import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import ConfirmDialog from '~/components/ConfirmDialog.vue'
import { useConfirm } from '~/composables/useConfirm'

/**
 * The dialog's focus watcher read `state.open` on the ref itself, which is always
 * undefined, so it never fired: focus stayed wherever it was when a confirm opened.
 * It now moves focus into the dialog, and a danger confirm focuses Cancel so that
 * Enter can never land on the destructive action.
 */
async function openConfirm(danger: boolean) {
  const { confirm, handleCancel } = useConfirm()
  const answer = confirm({
    title: 'Delete All Memories',
    message: 'This cannot be undone.',
    confirmLabel: 'Delete',
    danger,
  })
  await nextTick()
  await flushPromises()
  return { answer, handleCancel }
}

describe('ConfirmDialog', () => {
  it('focuses Cancel, not the destructive action, on a danger confirm', async () => {
    const wrapper = mount(ConfirmDialog, { attachTo: document.body })
    const { answer, handleCancel } = await openConfirm(true)

    expect(document.activeElement?.textContent?.trim()).toBe('Cancel')

    handleCancel()
    await expect(answer).resolves.toBe(false)
    wrapper.unmount()
  })

  it('focuses the confirm button on an ordinary confirm', async () => {
    const wrapper = mount(ConfirmDialog, { attachTo: document.body })
    const { answer, handleCancel } = await openConfirm(false)

    expect(document.activeElement?.textContent?.trim()).toBe('Delete')

    handleCancel()
    await expect(answer).resolves.toBe(false)
    wrapper.unmount()
  })
})
