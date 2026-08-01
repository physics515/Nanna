/**
 * Gate a native provider's model entries on DAEMON-reported availability.
 *
 * The daemon's LLM router is what actually routes a chat request; the GUI's
 * local credential state can disagree with it (e.g. an OAuth login the daemon
 * hasn't registered). When the daemon has answered (`daemonProviders` is an
 * array), it is authoritative — even an empty array means "offer nothing
 * native". Local credential state only decides when the daemon could not be
 * queried at all (`null`), where hiding everything would just blank the
 * picker without making chat any more routable.
 */
export function isProviderAvailable(
  daemonProviders: string[] | null,
  provider: string,
  localCredential: boolean,
): boolean {
  if (daemonProviders !== null) return daemonProviders.includes(provider)
  return localCredential
}
