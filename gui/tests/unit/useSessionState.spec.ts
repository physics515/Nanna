import { ref } from 'vue'
import { useSessionState } from '~/composables/useSessionState'

/**
 * P18 "Diff presentation": a tool end records the edit's before/after view in
 * the live journal. A tool end with no open start — the GUI attached or
 * reconnected mid-call — records a fresh item, and it must keep the diff just
 * as the matched branch does; the daemon's shared writer keeps it on both.
 */
describe('useSessionState timelineToolEnd', () => {
  const diff = { start_line: 3, removed: ['old'], added: ['new'], truncated: false }

  it('keeps the diff when no open start matches', () => {
    const s = useSessionState(ref('spec-orphan-end'))
    s.timelineToolEnd('c1', 'edit_file', 'ok', true, 12, false, diff)
    expect(s.liveTimeline.value).toHaveLength(1)
    expect(s.liveTimeline.value[0]).toMatchObject({
      kind: 'tool',
      call_id: 'c1',
      output: 'ok',
      success: true,
      diff,
    })
  })

  it('keeps the diff when an open start matches', () => {
    const s = useSessionState(ref('spec-open-end'))
    s.setLiveTimeline([
      {
        kind: 'tool',
        call_id: 'c2',
        name: 'edit_file',
        input: null,
        output: null,
        success: null,
        duration_ms: null,
        tokens: null,
        total_tokens: null,
        at: new Date().toISOString(),
      },
    ])
    s.timelineToolEnd('c2', 'edit_file', 'ok', true, 5, false, diff)
    expect(s.liveTimeline.value).toHaveLength(1)
    expect(s.liveTimeline.value[0]).toMatchObject({ output: 'ok', diff })
  })

  it('records no diff unless one is given', () => {
    const s = useSessionState(ref('spec-no-diff'))
    s.timelineToolEnd('c3', 'read_file', 'text', true, 1)
    expect(s.liveTimeline.value[0]).toMatchObject({ call_id: 'c3', diff: null })
  })
})
