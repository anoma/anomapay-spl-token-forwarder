// Constants of the forwarder's interface with the adapter.

/**
 * Number of accounts every forwarder CPI segment starts with: the forwarder,
 * its config, the instructions sysvar, the event authority and the forwarder
 * again.
 */
export const FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS = 5;

/** Number of accounts in a wrap forwarder CPI segment (`buildWrapForwarderAccounts`). */
export const FORWARDER_WRAP_NUM_ACCOUNTS = FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS + 5;

/** Number of accounts in an unwrap forwarder CPI segment (`buildUnwrapForwarderAccounts`). */
export const FORWARDER_UNWRAP_NUM_ACCOUNTS = FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS + 4;

/**
 * Number of accounts in the forwarder CPI segment of a migration of `entries`
 * resources (`buildMigrateForwarderAccounts`): the head, the adapter's state
 * and the vulnerable logic refs, then three per entry.
 */
export function forwarderMigrateNumAccounts(entries: number): number {
  return FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS + 2 + 3 * entries;
}
