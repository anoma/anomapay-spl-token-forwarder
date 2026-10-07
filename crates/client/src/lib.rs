//! Client bindings for the AnomaPay SPL Token Forwarder: the instructions it
//! takes, the CPI segments a settlement passes it, its wire formats (the wrap
//! authorization message, the wrap and unwrap inputs) and its events. The
//! adapter's own bindings are anoma-pa-solana-client's.

pub mod accounts;
pub mod constants;
pub mod events;
pub mod input;
pub mod wrap_message;

#[cfg(feature = "solana")]
pub mod forwarder;
#[cfg(feature = "solana")]
pub mod pda;
#[cfg(feature = "solana")]
pub mod program_ids;

pub use accounts::{
    decode_config, decode_nonce_bitmap, AccountDecodeError, ConfigAccount, NonceBitmapAccount,
};
pub use constants::*;
pub use events::{
    decode_forwarder_event_instruction, EmergencyCallerSetEvent, EmergencyWithdrawEvent,
    ForwarderEvent, InitializedEvent, UnwrappedEvent, WrappedEvent,
};
pub use input::{
    decode_forwarder_input, encode_unwrap_forwarder_input, encode_wrap_forwarder_input,
    ForwarderInput, InputError, UnwrapInput, WrapInput, OP_UNWRAP, OP_WRAP,
};
pub use wrap_message::{sha256, WrapMessage, WRAP_MESSAGE_LEN};

#[cfg(feature = "solana")]
pub use forwarder::{
    build_unwrap_forwarder_accounts, build_wrap_forwarder_accounts, close_config_ix,
    close_escrow_ix, close_nonce_bitmaps_batch_ix, forward_emergency_call_ix,
    forwarder_settlement_lookup_keys, init_nonce_bitmap_ix, initialize_ix, nonce_word_index,
    reinitialize_ix, renounce_ownership_ix, set_emergency_caller_ix, transfer_ownership_ix,
    upgrade_ixs, version_ix, UPGRADE_COMPUTE_UNIT_LIMIT,
};
#[cfg(feature = "solana")]
pub use pda::{
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, derive_nonce_bitmap_pda,
};
#[cfg(feature = "solana")]
pub use program_ids::*;

/// The vendored IDL, which the unit tests check the hand-written items against.
#[cfg(test)]
mod idl {
    pub(crate) fn idl() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idl/spl_token_forwarder.json"
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
