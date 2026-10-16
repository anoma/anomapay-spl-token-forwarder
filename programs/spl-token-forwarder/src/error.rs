//! Error codes. Where a check has expected-versus-actual context, the
//! program logs it with `msg!` before returning, since Anchor errors carry
//! no parameters.

use anchor_lang::prelude::*;

/// Declares `ErrorCode`: the forwarder's errors, then those of the build
/// (the emergency-migrating build's, below). Anchor's IDL takes one error
/// enum per program, so a build's errors extend the forwarder's, after them.
macro_rules! error_codes {
    ($($build:tt)*) => {
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

    #[msg("Protocol Adapter not paused - cannot close the forwarder's accounts")]
    ProtocolAdapterNotPaused,

    #[msg("Invalid PA state account - does not match derived PDA from protocol_adapter")]
    InvalidPaState,

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

    /// `migrate_config` found no config in the previous release's layout.
    #[msg("The config is not in the previous release's layout")]
    NotPreviousLayout,

    /// An owner-only instruction found the config in a previous release's
    /// layout, which `migrate_config` rewrites.
    #[msg("The config is in a previous release's layout - run migrate_config first")]
    ConfigNotMigrated,

    $($build)*
}
    };
}

#[cfg(not(feature = "emergency-migrating"))]
error_codes! {}

// The emergency-migrating build's errors: the EVM draft's, then the accounts
// a migration reads.
#[cfg(feature = "emergency-migrating")]
error_codes! {
    /// The EVM draft's `LogicRefNotDenied`.
    #[msg("A denylist of the adapter lacks a vulnerable logic ref")]
    LogicRefNotDenied,

    /// The EVM draft's `LogicRefAlreadyDenied`.
    #[msg("A denylist of the adapter holds the new logic ref")]
    LogicRefAlreadyDenied,

    /// The EVM draft's `EmptyVulnerableLogicRefList`.
    #[msg("reinitialize lists no vulnerable logic ref")]
    EmptyVulnerableLogicRefList,

    /// The EVM draft's `LogicRefAlreadyVulnerable`.
    #[msg("The logic ref is listed as vulnerable already")]
    LogicRefAlreadyVulnerable,

    /// The EVM draft's `LogicRefNotVulnerable`.
    #[msg("The logic ref is not listed as vulnerable")]
    LogicRefNotVulnerable,

    /// The EVM draft's `NonExistingRoot`.
    #[msg("The root is not a historical root of the adapter")]
    NonExistingRoot,

    /// The EVM draft's `UnknownForwarder`.
    #[msg("The resource's label names neither this forwarder nor the legacy forwarder")]
    UnknownForwarder,

    /// The EVM draft's `EmptyMigrationBatch`.
    #[msg("The migration names no resource")]
    EmptyMigrationBatch,

    /// The EVM draft's `ResourceAlreadyConsumed`.
    #[msg("The adapter consumed the resource already")]
    ResourceAlreadyConsumed,

    /// The EVM draft's `ResourceAlreadyMigrated`.
    #[msg("The forwarder migrated the resource already")]
    ResourceAlreadyMigrated,

    #[msg("Migration record does not exist - create it with init_migration_record before the migration")]
    MigrationRecordMissing,

    #[msg("Invalid migration record PDA - doesn't match the nullifier's derived address")]
    InvalidMigrationRecordPda,

    #[msg("Invalid vulnerable logic refs PDA - doesn't match the derived address")]
    InvalidVulnerableLogicRefsPda,

    /// The adapter's `NullifierPdaMismatch`: the account a migration reads
    /// as the adapter's marker of a nullifier is not at its address.
    #[msg("Nullifier PDA mismatch - not the adapter's marker of the nullifier")]
    NullifierPdaMismatch,
}
