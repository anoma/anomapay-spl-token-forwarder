//! Error codes. Where a check has expected-versus-actual context, the
//! program logs it with `msg!` before returning, since Anchor errors carry
//! no parameters.

use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid input data")]
    InvalidInput,

    /// The EVM forwarder's `InvalidInputLength`: a wrap or unwrap operand of
    /// the wrong length.
    #[msg("Invalid input length")]
    InvalidInputLength,

    #[msg("Invalid token account data - too short or malformed")]
    InvalidTokenAccountData,

    #[msg("Insufficient remaining accounts - check logs for expected count")]
    InsufficientRemainingAccounts,

    #[msg("Invalid token program - expected SPL Token program ID")]
    InvalidTokenProgram,

    #[msg("Token account owner mismatch - the account does not belong to the party the transfer names")]
    WrongTokenAccountOwner,

    #[msg("Token account mint mismatch - the account does not hold the mint the transfer names")]
    WrongTokenAccountMint,

    #[msg("Unknown operation code")]
    UnknownOperation,

    /// The EVM forwarder's `ProtocolAdapterMismatch`: `forward_call`'s caller
    /// is not the protocol adapter.
    #[msg("The caller is not the protocol adapter")]
    ProtocolAdapterMismatch,

    /// The signer is not the emergency committee or the emergency caller an
    /// emergency instruction requires.
    #[msg("Unauthorized caller")]
    UnauthorizedCaller,

    /// The EVM forwarder's `LogicRefMismatch`: the calling resource's logic
    /// ref is not the one this forwarder serves.
    #[msg("The calling resource's logic ref is not this forwarder's")]
    LogicRefMismatch,

    #[msg("Signature deadline has expired")]
    DeadlineExpired,

    #[msg("Nonce has already been used (replay attack prevented)")]
    NonceAlreadyUsed,

    #[msg("Ed25519 instruction not found at specified index")]
    Ed25519InstructionNotFound,

    #[msg("Invalid Ed25519 instruction format")]
    InvalidEd25519Instruction,

    #[msg("Ed25519 public key mismatch")]
    Ed25519PubkeyMismatch,

    #[msg("Ed25519 message mismatch")]
    Ed25519MessageMismatch,

    #[msg("Emergency caller already set")]
    EmergencyCallerAlreadySet,

    #[msg("Emergency caller not set")]
    EmergencyCallerNotSet,

    #[msg("Protocol Adapter not paused - cannot perform emergency operations")]
    ProtocolAdapterNotPaused,

    #[msg("Invalid PA state account - does not match derived PDA from protocol_adapter")]
    InvalidPaState,

    /// A zero emergency committee or emergency caller.
    #[msg("Zero address not allowed")]
    ZeroAddressNotAllowed,

    /// The EVM forwarder's `ZeroProtocolAdapterNotAllowed`.
    #[msg("Zero protocol adapter not allowed")]
    ZeroProtocolAdapterNotAllowed,

    /// The EVM forwarder's `ZeroLogicRefNotAllowed`.
    #[msg("Zero logic ref not allowed")]
    ZeroLogicRefNotAllowed,

    #[msg("Invalid escrow authority")]
    InvalidEscrowAuthority,

    #[msg("Invalid nonce bitmap PDA - doesn't match derived address")]
    InvalidNonceBitmapPda,

    #[msg(
        "Nonce bitmap account does not exist - create it with init_nonce_bitmap before the wrap"
    )]
    NonceBitmapMissing,

    #[msg("Token transfer failed - check the SPL Token error in the logs")]
    TokenTransferFailed,

    #[msg("Unwrap recipient is the escrow authority - the tokens would never leave escrow")]
    UnwrapToEscrow,

    #[msg("The config is already at this build's CONFIG_VERSION - rotating the logic ref takes a build that raises it")]
    InvalidInitialization,

    /// OpenZeppelin Ownable's `OwnableUnauthorizedAccount`: the signer is not the owner.
    #[msg("The signer is not the forwarder's owner")]
    OwnableUnauthorizedAccount,

    /// OpenZeppelin Ownable's `OwnableInvalidOwner`: the zero key cannot be made the owner.
    #[msg("The zero key cannot be the owner")]
    OwnableInvalidOwner,

    #[msg("Buffer is not a loader buffer holding a program")]
    InvalidUpgradeBuffer,

    #[msg("Unauthorized: the signer does not hold the authority this instruction requires")]
    Unauthorized,
}
