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

/**
 * P19 interjection placement: a message the user sends while the model is
 * still streaming must land AFTER the assistant content streamed so far, and
 * text that arrives afterwards must open a new segment below it rather than
 * extend the one before it. Before this, the page pushed the interjection
 * into the message list — rendered wholesale above the live journal — so it
 * sat beside the previous reply while the run kept streaming underneath.
 */
describe('useSessionState timelineUserMessage', () => {
  it('journals the interjection after the streamed content and before later chunks', () => {
    const s = useSessionState(ref('spec-interject-order'))
    s.timelineAppendSegment('text', 'Working on it')
    s.timelineToolStart({ id: 'c1', name: 'exec', input: {}, output: '', success: false, duration_ms: 0 })
    s.timelineAppendSegment('text', ' still going')

    const entry = s.timelineUserMessage('actually, use bash')
    expect(entry).not.toBeNull()

    // Text after the interjection is a NEW segment — appending onto the
    // pre-interjection segment would render the reply above the question.
    s.timelineAppendSegment('text', 'Switching to bash')

    expect(s.liveTimeline.value.map(i => i.kind)).toEqual(['text', 'tool', 'text', 'user', 'text'])
    expect(s.liveTimeline.value[2]).toMatchObject({ kind: 'text', content: ' still going' })
    expect(s.liveTimeline.value[3]).toMatchObject({ kind: 'user', content: 'actually, use bash' })
    expect(s.liveTimeline.value[4]).toMatchObject({ kind: 'text', content: 'Switching to bash' })
  })

  it('retracts the journaled interjection when the send fails', () => {
    const s = useSessionState(ref('spec-interject-retract'))
    s.timelineAppendSegment('text', 'Working')
    const entry = s.timelineUserMessage('never reached the daemon')!
    s.timelineRemove(entry)
    expect(s.liveTimeline.value.map(i => i.kind)).toEqual(['text'])
    // Removing an entry that is not there is a no-op, not a crash.
    s.timelineRemove(entry)
    expect(s.liveTimeline.value).toHaveLength(1)
  })

  it('does not journal an interjection for a session with no state', () => {
    const s = useSessionState(ref<string | null>(null))
    expect(s.timelineUserMessage('orphan')).toBeNull()
  })
})
