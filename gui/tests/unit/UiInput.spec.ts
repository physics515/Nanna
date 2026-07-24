import { mount } from '@vue/test-utils'
import UiInput from '~/components/ui/input.vue'

/**
 * `size` has to be a *declared prop*. HTML's `size` on an `<input>` takes a positive integer, so
 * an undeclared `size="sm"` falls through to the DOM and throws `IndexSizeError`, which Vue turns
 * into a warning and swallows — five call sites shipped that way. These assert the prop is
 * consumed (never reaches the element) and that it actually changes the rendered classes.
 */
describe('UiInput', () => {
  it('never forwards size to the DOM element', () => {
    for (const size of ['sm', 'default', 'lg'] as const) {
      const input = mount(UiInput, { props: { size } }).get('input')
      expect(input.attributes('size')).toBeUndefined()
    }
  })

  it('maps each size to distinct height classes', () => {
    const classesFor = (size: 'sm' | 'default' | 'lg') =>
      mount(UiInput, { props: { size } }).get('input').classes()

    expect(classesFor('sm')).toContain('h-8')
    expect(classesFor('default')).toContain('h-10')
    expect(classesFor('lg')).toContain('h-12')
  })

  it('defaults to the medium size when none is given', () => {
    expect(mount(UiInput).get('input').classes()).toContain('h-10')
  })

  it('lets a caller class win over the variant', () => {
    // tailwind-merge resolves the conflict in favour of the explicit class.
    const classes = mount(UiInput, { props: { size: 'sm', class: 'h-12' } }).get('input').classes()
    expect(classes).toContain('h-12')
    expect(classes).not.toContain('h-8')
  })

  it('still emits update:modelValue', async () => {
    const wrapper = mount(UiInput, { props: { modelValue: '' } })
    await wrapper.get('input').setValue('typed')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['typed'])
  })
})
