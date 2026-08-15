import {
  describeMarkers,
  hasWorkspaceMarker,
  requiresStandardFile,
  type WorkspaceValidity,
} from '~/lib/workspaceMarkers'

/**
 * The create dialog must require what the BACKEND requires: one workspace
 * marker on disk. It used to demand at least one standard context file no
 * matter what, so "create workspace" wrote AGENTS.md into repositories that
 * were already valid workspaces — a toll the daemon never charged.
 */
function validity(overrides: Partial<WorkspaceValidity> = {}): WorkspaceValidity {
  return {
    exists: true,
    is_valid: false,
    has_readme: false,
    has_agents: false,
    has_contributing: false,
    has_roadmap: false,
    has_git: false,
    has_manifest: false,
    ...overrides,
  }
}

describe('workspace create requirements', () => {
  it('requires no standard file when the folder already carries a marker', () => {
    const git = validity({ is_valid: true, has_git: true })
    expect(hasWorkspaceMarker(git)).toBe(true)
    expect(requiresStandardFile(git)).toBe(false)
  })

  it('requires one file for a bare, markerless folder', () => {
    const bare = validity()
    expect(hasWorkspaceMarker(bare)).toBe(false)
    expect(requiresStandardFile(bare)).toBe(true)
  })

  it('treats an unchecked folder as markerless (nothing known yet)', () => {
    expect(hasWorkspaceMarker(null)).toBe(false)
    expect(requiresStandardFile(null)).toBe(true)
  })

  it('accepts a manifest or a plain README as the marker, with zero standard files', () => {
    const cargo = validity({ is_valid: true, has_manifest: true })
    const readme = validity({ is_valid: true, has_readme: true })
    expect(requiresStandardFile(cargo)).toBe(false)
    expect(requiresStandardFile(readme)).toBe(false)
  })

  it('names the markers it found so the dialog can say why', () => {
    expect(describeMarkers(validity({ is_valid: true, has_git: true })))
      .toBe('.git')
    expect(describeMarkers(validity({ is_valid: true, has_git: true, has_manifest: true })))
      .toBe('.git and a project manifest')
    expect(describeMarkers(validity({
      is_valid: true,
      has_git: true,
      has_manifest: true,
      has_readme: true,
    }))).toBe('.git, a project manifest and README.md')
  })

  it('falls back to a generic phrase when no marker is itemised', () => {
    // `.nanna` alone is a marker the validity check does not break out.
    expect(describeMarkers(validity({ is_valid: true }))).toBe('an existing project marker')
    expect(describeMarkers(null)).toBe('an existing project marker')
  })
})
