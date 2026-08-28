/**
 * What the BACKEND requires of a workspace directory.
 *
 * The daemon's bar for "this folder is a workspace" is one workspace marker on
 * disk — `.git`, `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`,
 * `README.md`, `AGENTS.md`, `ROADMAP.md`, `.nanna` (crates/nanna-workspace
 * discovery; the daemon's Open handler only creates `.nanna` when NO marker is
 * present). `check_workspace_validity` reports exactly that check as
 * `is_valid`.
 *
 * The create dialog used to demand at least one standard context file
 * regardless, which meant "create workspace" wrote AGENTS.md into repositories
 * that were already valid workspaces — a requirement the backend never made.
 */

/** Shape returned by the `check_workspace_validity` command. */
export interface WorkspaceValidity {
  exists: boolean
  /** True iff at least one workspace marker is present on disk. */
  is_valid: boolean
  has_readme: boolean
  has_agents: boolean
  has_contributing: boolean
  has_roadmap: boolean
  has_git: boolean
  has_manifest: boolean
}

/**
 * The four standard context files, as keys into both `WorkspaceValidity` and
 * the workspace rows the list renders.
 *
 * A literal union rather than `string`: the UI indexes those objects by these
 * keys (`ws[file.key]`), and a `string` key is not provably a member of either
 * shape — which is what made every such read an implicit `any`.
 */
export type ContextFileKey =
  | 'has_readme'
  | 'has_agents'
  | 'has_contributing'
  | 'has_roadmap'

/** Does the folder already carry a marker the backend recognises? */
export function hasWorkspaceMarker(validity: WorkspaceValidity | null): boolean {
  return validity?.is_valid === true
}

/**
 * Must the user pick at least one standard file?
 *
 * Only for a bare, markerless folder — without one file nothing on disk would
 * mark it as a workspace at all. A marked folder needs nothing.
 */
export function requiresStandardFile(validity: WorkspaceValidity | null): boolean {
  return !hasWorkspaceMarker(validity)
}

/**
 * Names the markers actually found, so the dialog can state WHY nothing is
 * required rather than just asserting it.
 */
export function describeMarkers(validity: WorkspaceValidity | null): string {
  const fallback = 'an existing project marker'
  if (!validity) return fallback
  const found: string[] = []
  if (validity.has_git) found.push('.git')
  if (validity.has_manifest) found.push('a project manifest')
  if (validity.has_agents) found.push('AGENTS.md')
  if (validity.has_readme) found.push('README.md')
  if (validity.has_roadmap) found.push('ROADMAP.md')
  if (validity.has_contributing) found.push('CONTRIBUTING.md')
  if (found.length === 0) return fallback
  if (found.length === 1) return found[0]!
  return `${found.slice(0, -1).join(', ')} and ${found[found.length - 1]}`
}
