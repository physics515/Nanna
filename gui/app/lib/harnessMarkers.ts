/**
 * Harness plumbing that must never render as conversation.
 *
 * The long-horizon harness verdicts unchecked work on a literal
 * `TASK COMPLETE` line in the step output (see nanna-agent harness).
 * The daemon strips it before persisting a message; this covers the live
 * stream — where the marker arrives as ordinary deltas — and any history
 * persisted before the daemon-side strip existed. Only whole marker lines
 * are dropped; inline mentions are left alone.
 */
export function stripHarnessMarkers(text: string): string {
  if (!/task complete/i.test(text)) return text
  return text
    .split('\n')
    .filter(line => line.trim().toLowerCase() !== 'task complete')
    .join('\n')
}

/**
 * Does this segment have anything to SHOW once the plumbing is out?
 *
 * The single predicate every card gate uses — assistant bubbles, streaming
 * bubble, thinking cards. A segment that is only whitespace or only harness
 * markers draws no card at all: an empty card reads as the model failing to
 * answer when in fact it was working.
 */
export function hasRenderableText(text: string | null | undefined): boolean {
  return stripHarnessMarkers(text ?? '').trim().length > 0
}
