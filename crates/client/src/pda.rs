//! Program-derived-address helpers for the SPL Token Forwarder.
//!
//! These mirror the seed schemas baked into the program. They are pure
//! functions: same inputs always produce the same `(Pubkey, bump)` pair.

use solana_pubkey::Pubkey;

/// Derive the forwarder's global config PDA. Seed: `["config"]`.
pub fn derive_forwarder_config_pda(forwarder_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config"], forwarder_program)
}

/// Derive the forwarder's escrow authority. Seed: `["escrow"]`.
///
/// One PDA owns every mint's escrow: each escrow is the Associated Token
/// Account of this authority and the mint, as the EVM forwarder holds every
/// token at its own address. It is also the delegate users name in their SPL
/// `Approve` instruction before a wrap.
pub fn derive_forwarder_escrow_authority(forwarder_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"escrow"], forwarder_program)
}

/// Derive the forwarder's nonce bitmap PDA for a (user, word_index) pair.
///
/// Seed: `["nonce_bitmap", user, word_index_le]`. `word_index = nonce / 256`.
pub fn derive_nonce_bitmap_pda(
    forwarder_program: &Pubkey,
    user: &Pubkey,
    word_index: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"nonce_bitmap", user.as_ref(), &word_index.to_le_bytes()],
        forwarder_program,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program_ids::FORWARDER_PROGRAM_ID;

    #[test]
    fn forwarder_escrow_authority_matches_the_devnet_forwarder() {
        // Independent pin: the devnet forwarder's escrow authority, as
        // docs/DEVNET_DEPLOYMENT.md records it.
        let (authority, bump) = derive_forwarder_escrow_authority(&FORWARDER_PROGRAM_ID);
        assert_eq!(
            authority.to_string(),
            "G78SQtzYuo4YKDEECzh25rckXeJFjLMXy44iWKKG5rDG"
        );
        assert_eq!(bump, 255);
    }
}
