// Constants of the forwarder's interface with the adapter.

/** Number of accounts in a wrap forwarder CPI segment (`buildWrapForwarderAccounts`). */
export const FORWARDER_WRAP_NUM_ACCOUNTS = 10;

/** Number of accounts in an unwrap forwarder CPI segment (`buildUnwrapForwarderAccounts`). */
export const FORWARDER_UNWRAP_NUM_ACCOUNTS = 9;

/**
 * Return data of a successful forwarder call: the one byte the SPL token
 * forwarder returns and the resource's external call expects as output.
 */
export const FORWARDER_RESULT_SUCCESS = 1;
