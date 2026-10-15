// Constants of the forwarder's interface with the adapter.

/** Number of accounts in a wrap forwarder CPI segment (`buildWrapForwarderAccounts`). */
export const FORWARDER_WRAP_NUM_ACCOUNTS = 10;

/** Number of accounts in an unwrap forwarder CPI segment (`buildUnwrapForwarderAccounts`). */
export const FORWARDER_UNWRAP_NUM_ACCOUNTS = 9;

/**
 * Number of accounts in the forwarder CPI segment of a migration of `entries`
 * resources (`buildMigrateForwarderAccounts`): the segment head, the adapter's
 * state and the vulnerable logic refs, then three per entry.
 */
export function forwarderMigrateNumAccounts(entries: number): number {
  return 7 + 3 * entries;
}
