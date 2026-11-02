// Constants of the forwarder's interface with the adapter.

import { idl } from "./idl.js";

/** The accounts the IDL declares for the forwarder's `forward_call`, which the adapter calls. */
export const FORWARD_CALL_ACCOUNTS = (() => {
  const forwardCall = idl.instructions.find((ix) => ix.name === "forward_call");
  if (forwardCall === undefined) throw new Error(`the IDL of ${idl.metadata.name} declares no forward_call`);
  return forwardCall.accounts;
})();

/**
 * Number of accounts every forwarder CPI segment starts with: the forwarder,
 * the program the adapter calls, then the accounts of its `forward_call`.
 */
export const FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS = 1 + FORWARD_CALL_ACCOUNTS.length;

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

/**
 * The compute-unit limit an upgrade asks for: the most a transaction may have
 * (Agave's `MAX_COMPUTE_UNIT_LIMIT`), as the Rust crate's
 * `UPGRADE_COMPUTE_UNIT_LIMIT`. The upgrade hashes the whole buffer, so its
 * cost grows with the program's code.
 */
export const UPGRADE_COMPUTE_UNIT_LIMIT = 1_400_000;
