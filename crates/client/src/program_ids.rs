//! The forwarder's program id.

use solana_pubkey::{pubkey, Pubkey};

/// SPL Token Forwarder program ID: the V2 forwarder's declared id, under which
/// this repository builds and deploys it.
pub const FORWARDER_PROGRAM_ID: Pubkey = pubkey!("BsfuXpxw8oCmZXnYijyQkUYNcCnuskFZbYizmWLnpSU7");

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
