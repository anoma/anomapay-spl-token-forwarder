//! The program identifiers a wrap or unwrap names.

use solana_pubkey::{pubkey, Pubkey};

/// SPL Token Forwarder program ID: the V2 forwarder's declared id, under which
/// this repository builds and deploys it.
pub const FORWARDER_PROGRAM_ID: Pubkey = pubkey!("BsfuXpxw8oCmZXnYijyQkUYNcCnuskFZbYizmWLnpSU7");

/// Solana's native ed25519 signature-verification program. Used to carry verified
/// wrap-authorization signatures into the settle transaction.
pub const ED25519_PROGRAM_ID: Pubkey = pubkey!("Ed25519SigVerify111111111111111111111111111");

/// Solana's `Instructions` sysvar (used by the forwarder to introspect the
/// ed25519-verify instruction at `ed25519_ix_index`).
pub const INSTRUCTIONS_SYSVAR_ID: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarder_program_id_matches_the_vendored_idl() {
        let idl: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idl/spl_token_forwarder.json"
        )))
        .unwrap();
        assert_eq!(
            idl["address"].as_str().unwrap(),
            FORWARDER_PROGRAM_ID.to_string()
        );
    }
}
