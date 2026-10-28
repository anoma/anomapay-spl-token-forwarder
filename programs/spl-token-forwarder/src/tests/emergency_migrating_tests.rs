//! Tests for the emergency-migrating build's accounts and the tokens it holds.

use crate::emergency_migrating::{
    holds_the_tokens_of, MigrationRecord, VulnerableLogicRefs, MIGRATION_RECORD_SEED,
    VULNERABLE_LOGIC_REFS_PDA, VULNERABLE_LOGIC_REFS_SEED,
};
use anchor_lang::prelude::Pubkey;
use anchor_lang::AccountSerialize;

/// Mirrors test_migrate_accepts_a_resource_with_the_legacy_forwarder_in_its_label
/// and test_migrate_reverts_on_the_zero_forwarder_on_a_chain_without_a_legacy_forwarder:
/// the forwarder holds the tokens of its own resources and of the legacy
/// forwarder's, and on a cluster without one the zero key names none.
#[test]
fn the_forwarder_holds_its_own_and_the_legacy_forwarders_tokens() {
    let (legacy, other) = (Pubkey::new_unique(), Pubkey::new_unique());
    assert!(holds_the_tokens_of(&crate::ID, &legacy));
    assert!(holds_the_tokens_of(&legacy, &legacy));
    assert!(!holds_the_tokens_of(&other, &legacy));

    let none = Pubkey::default();
    assert!(holds_the_tokens_of(&crate::ID, &none));
    assert!(!holds_the_tokens_of(&none, &none));
    assert!(!holds_the_tokens_of(&other, &none));
}

/// The compile-time address of the vulnerable logic refs is the canonical
/// PDA `reinitialize` creates.
#[test]
fn vulnerable_logic_refs_pda_is_the_canonical_address() {
    let (canonical, _) = Pubkey::find_program_address(&[VULNERABLE_LOGIC_REFS_SEED], &crate::ID);
    assert_eq!(VULNERABLE_LOGIC_REFS_PDA, canonical);
}

/// `space(n)` holds `n` logic refs in the account's layout.
#[test]
fn vulnerable_logic_refs_space_holds_its_count() {
    for count in [0, 1, 5] {
        let mut data = Vec::new();
        VulnerableLogicRefs {
            logic_refs: vec![[0; 32]; count],
        }
        .try_serialize(&mut data)
        .unwrap();
        assert_eq!(
            data.len(),
            VulnerableLogicRefs::space(count),
            "{count} refs"
        );
    }
}

/// A record is recognized at its nullifier's canonical address under its
/// stored bump, and nowhere else.
#[test]
fn a_migration_record_is_at_its_nullifiers_canonical_address() {
    let nullifier = [3u8; 32];
    let (address, bump) =
        Pubkey::find_program_address(&[MIGRATION_RECORD_SEED, &nullifier], &crate::ID);
    let record = MigrationRecord {
        migrated: false,
        bump,
    };
    assert!(record.is_at(&address, &nullifier));
    assert!(!record.is_at(&address, &[4u8; 32]));
    assert!(!record.is_at(&Pubkey::new_unique(), &nullifier));
    assert_eq!(MigrationRecord::ACCOUNT_SIZE, 8 + 1 + 1);
}
