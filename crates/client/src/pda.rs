//! Program-derived-address helpers for the SPL Token Forwarder, from the
//! seeds its IDLs declare (`generated::*::constants`): the generated client
//! takes every account as an address and derives none. They are pure
//! functions: same inputs always produce the same `(Pubkey, bump)` pair.

use solana_pubkey::Pubkey;

use crate::generated::spl_token_forwarder::constants::{
    CONFIG_SEED, ESCROW_SEED, NONCE_BITMAP_SEED,
};

/// Derive the forwarder's global config PDA. Seed: `[CONFIG_SEED]`.
pub fn derive_forwarder_config_pda(forwarder_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CONFIG_SEED], forwarder_program)
}

/// Derive the forwarder's escrow authority. Seed: `[ESCROW_SEED]`.
///
/// One PDA owns every mint's escrow: each escrow is the Associated Token
/// Account of this authority and the mint, as the EVM forwarder holds every
/// token at its own address. It is also the delegate users name in their SPL
/// `Approve` instruction before a wrap.
pub fn derive_forwarder_escrow_authority(forwarder_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[ESCROW_SEED], forwarder_program)
}

/// Derive the forwarder's nonce bitmap PDA for a (user, word_index) pair.
///
/// Seed: `[NONCE_BITMAP_SEED, user, word_index_le]`, `word_index` being
/// `nonce / NONCES_PER_WORD`.
pub fn derive_nonce_bitmap_pda(
    forwarder_program: &Pubkey,
    user: &Pubkey,
    word_index: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[NONCE_BITMAP_SEED, user.as_ref(), &word_index.to_le_bytes()],
        forwarder_program,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emergency_migrating::derive_vulnerable_logic_refs_pda;
    use crate::forwarder::tests::idl_address;
    use crate::generated::spl_token_forwarder::ID;

    #[test]
    fn forwarder_escrow_authority_is_the_configured_forwarders() {
        // Independent pin: the configured forwarder's escrow authority, as
        // `solana find-program-derived-address <forwarder> string:escrow`
        // derives it.
        let (authority, bump) = derive_forwarder_escrow_authority(&ID);
        assert_eq!(
            authority.to_string(),
            "A3srA2cw6o11TCSyMpBbnpLVnu6RLe6eGZEFz3d2m8h1"
        );
        assert_eq!(bump, 255);
    }

    /// Every instruction account of either IDL that one of the helpers
    /// above derives, and whose address the IDL determines, is the helper's
    /// address.
    #[test]
    fn the_forwarder_pdas_are_the_ones_its_idls_declare() {
        let helpers = [
            ("config", derive_forwarder_config_pda(&ID).0),
            ("escrow_authority", derive_forwarder_escrow_authority(&ID).0),
            (
                "vulnerable_logic_refs",
                derive_vulnerable_logic_refs_pda(&ID).0,
            ),
        ];
        let mut checked = std::collections::BTreeSet::new();
        for idl in [crate::idl::idl(), crate::idl::emergency_migrating_idl()] {
            for instruction in idl["instructions"].as_array().unwrap() {
                for account in instruction["accounts"].as_array().unwrap() {
                    let Some((name, derived)) =
                        helpers.iter().find(|(name, _)| account["name"] == *name)
                    else {
                        continue;
                    };
                    let declared = idl_address(account, &ID).unwrap_or_else(|| {
                        panic!(
                            "{}.{name}: the IDL determines no address",
                            instruction["name"]
                        )
                    });
                    println!("{}.{name}: {declared}", instruction["name"]);
                    assert_eq!(*derived, declared, "{}.{name}", instruction["name"]);
                    checked.insert(*name);
                }
            }
        }
        assert_eq!(
            checked,
            helpers.iter().map(|(name, _)| *name).collect(),
            "every helper is checked against some IDL account"
        );
    }
}
