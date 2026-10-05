// Constants of the forwarder's interface with the adapter.

/**
 * Number of accounts in a wrap forwarder CPI segment.
 *
 * Ordering: `[forwarder_program, config_pda, ix_sysvar, event_authority,
 * forwarder_program, user_ata, escrow_ata, escrow_authority, nonce_bitmap_pda,
 * token_program]`. The event authority and the program let the forwarder emit
 * its events as CPI events. The nonce bitmap must already exist: the adapter's
 * CPI carries no signer that could pay for creating it, so a wrap on a word
 * without a bitmap is preceded by `init_nonce_bitmap`.
 */
export const FORWARDER_WRAP_NUM_ACCOUNTS = 10;

/**
 * Number of accounts in an unwrap forwarder CPI segment.
 *
 * Ordering: `[forwarder_program, config_pda, ix_sysvar, event_authority,
 * forwarder_program, escrow_ata, recipient_ata, escrow_authority, token_program]`.
 */
export const FORWARDER_UNWRAP_NUM_ACCOUNTS = 9;

/**
 * Return data of a successful forwarder call: the one byte the SPL token
 * forwarder returns and the resource's external call expects as output.
 */
export const FORWARDER_RESULT_SUCCESS = 1;
