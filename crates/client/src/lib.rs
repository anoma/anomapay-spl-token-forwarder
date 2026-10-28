//! Client bindings for the AnomaPay SPL Token Forwarder. Its id, accounts,
//! events, constants, errors and instruction accounts and arguments are
//! generated from its IDLs ([`generated`]); the rest is what the IDLs do not
//! describe: the instruction builders over the generated code with the PDAs
//! they derive, the CPI segments a settlement passes the forwarder, its wire
//! formats (the wrap authorization message, the wrap, unwrap and migrate
//! inputs), and the emergency-migrating build's `VulnerableLogicRefs`
//! account. The emergency-migrating build's instructions are in
//! [`emergency_migrating`]. The adapter's own bindings are
//! anoma-pa-solana-client's; its `decode_cpi_event` reads the generated
//! events.

pub mod accounts;
pub mod constants;
pub mod generated;
pub mod input;
pub mod wrap_message;

#[cfg(feature = "solana")]
pub mod emergency_migrating;
#[cfg(feature = "solana")]
pub mod forwarder;
#[cfg(feature = "solana")]
pub mod pda;
#[cfg(feature = "solana")]
pub mod settlement;
#[cfg(feature = "rpc")]
pub mod submitter;

pub use accounts::{decode_vulnerable_logic_refs, AccountDecodeError};
pub use constants::*;
pub use input::{
    decode_forwarder_input, encode_migrate_forwarder_input, encode_unwrap_forwarder_input,
    encode_wrap_forwarder_input, ForwarderInput, InputError, MigrateEntry, MigrateInput,
    UnwrapInput, WrapInput, OP_WRAP,
};
pub use wrap_message::{sha256, WrapMessage, WRAP_MESSAGE_LEN};

#[cfg(feature = "solana")]
pub use forwarder::{
    build_unwrap_forwarder_accounts, build_wrap_forwarder_accounts, close_config_ix,
    close_escrow_ix, close_nonce_bitmaps_batch_ix, init_nonce_bitmap_ix, initialize_ix,
    nonce_word_index, reinitialize_ix, renounce_ownership_ix, transfer_ownership_ix, upgrade_ixs,
    version_ix, UPGRADE_COMPUTE_UNIT_LIMIT,
};
#[cfg(feature = "solana")]
pub use pda::{
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, derive_nonce_bitmap_pda,
};
#[cfg(feature = "solana")]
pub use settlement::{settlement_calls, CallsError};

/// The vendored IDLs, which the unit tests of the instruction builders and
/// the PDA helpers check them against.
#[cfg(all(test, feature = "solana"))]
mod idl {
    pub(crate) fn idl() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idls/spl_token_forwarder.json"
        )))
        .unwrap()
    }

    /// The emergency-migrating build's IDL.
    pub(crate) fn emergency_migrating_idl() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idls/emergency_migrating_spl_token_forwarder.json"
        )))
        .unwrap()
    }
}
