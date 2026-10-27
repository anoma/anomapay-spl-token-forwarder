//! Client bindings for the AnomaPay SPL Token Forwarder: the instructions it
//! takes, the CPI segments a settlement passes it, its wire formats (the wrap
//! authorization message, the wrap, unwrap and migrate inputs) and its
//! events. The emergency-migrating build's instructions are in
//! [`emergency_migrating`]. The adapter's own bindings are
//! anoma-pa-solana-client's.

pub mod accounts;
pub mod constants;
pub mod events;
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
pub mod program_ids;
#[cfg(feature = "solana")]
pub mod settlement;
#[cfg(feature = "rpc")]
pub mod submitter;

pub use accounts::{
    decode_config, decode_migration_record, decode_nonce_bitmap, decode_vulnerable_logic_refs,
    AccountDecodeError, ConfigAccount, MigrationRecordAccount, NonceBitmapAccount,
};
pub use constants::*;
pub use events::{decode_forwarder_event_instruction, ForwarderEvent};
pub use input::{
    decode_forwarder_input, encode_migrate_forwarder_input, encode_unwrap_forwarder_input,
    encode_wrap_forwarder_input, ForwarderInput, InputError, MigrateEntry, MigrateInput,
    UnwrapInput, WrapInput, OP_MIGRATE, OP_UNWRAP, OP_WRAP,
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
pub use program_ids::*;
#[cfg(feature = "solana")]
pub use settlement::{settlement_calls, CallsError};

/// The vendored IDL, which the unit tests check the hand-written items against.
#[cfg(test)]
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

    /// The fields of the IDL's type `name`.
    pub(crate) fn fields<'a>(idl: &'a serde_json::Value, name: &str) -> &'a [serde_json::Value] {
        idl["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("the IDL declares no type {name}"))["type"]["fields"]
            .as_array()
            .unwrap()
    }
}
