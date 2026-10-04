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
pub mod ata;
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
pub use ata::create_ata_idempotent_ix;
#[cfg(feature = "solana")]
pub use forwarder::{
    build_unwrap_forwarder_accounts, build_wrap_forwarder_accounts, init_nonce_bitmap_ix,
    initialize_ix, nonce_word_index,
};
#[cfg(feature = "solana")]
pub use pda::{
    derive_associated_token_address, derive_forwarder_config_pda,
    derive_forwarder_escrow_authority, derive_nonce_bitmap_pda,
};
#[cfg(feature = "solana")]
pub use program_ids::*;
