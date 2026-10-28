//! Tests for the emergency-migrating build's wire format and accounts.

use crate::emergency_migrating::{
    holds_the_tokens_of, MigrateInput, MigrationRecord, VulnerableLogicRefs, MIGRATION_RECORD_SEED,
    OP_MIGRATE, VULNERABLE_LOGIC_REFS_PDA, VULNERABLE_LOGIC_REFS_SEED,
};
use anchor_lang::prelude::Pubkey;
use anchor_lang::{AccountDeserialize, AccountSerialize, AnchorDeserialize};
use anomapay_spl_token_forwarder_client as client;
use client::generated::emergency_migrating_spl_token_forwarder as generated;

/// The program parses exactly what the client encodes, the bytes the
/// trigger's logic commits: the op byte, then the Borsh `MigrateInput`.
#[test]
fn migrate_input_parses_the_client_encoding() {
    let entry = |i: u8| client::MigrateEntry {
        nullifier: [10 + i; 32],
        commitment_tree_root: [20 + i; 32],
        vulnerable_logic_ref: [30 + i; 32],
        forwarder: [40 + i; 32],
    };
    let encoded = client::encode_migrate_forwarder_input(&client::MigrateInput {
        token_mint: [1; 32],
        quantity: 300,
        entries: vec![entry(0), entry(1)],
    });
    let (&op, operand) = encoded.split_first().unwrap();
    assert_eq!(op, OP_MIGRATE);
    assert_eq!(generated::constants::OP_MIGRATE, OP_MIGRATE);

    let parsed = MigrateInput::try_from_slice(operand).unwrap();
    assert_eq!(parsed.token_mint, Pubkey::new_from_array([1; 32]));
    assert_eq!(parsed.quantity, 300);
    assert_eq!(parsed.entries.len(), 2);
    for (i, parsed) in parsed.entries.iter().enumerate() {
        let expected = entry(i as u8);
        assert_eq!(parsed.nullifier, expected.nullifier);
        assert_eq!(parsed.commitment_tree_root, expected.commitment_tree_root);
        assert_eq!(parsed.vulnerable_logic_ref, expected.vulnerable_logic_ref);
        assert_eq!(parsed.forwarder, Pubkey::new_from_array(expected.forwarder));
    }

    // Borsh refuses a byte more or less, as the EVM draft's length check.
    assert!(MigrateInput::try_from_slice(&operand[..operand.len() - 1]).is_err());
    assert!(MigrateInput::try_from_slice(&[operand, &[0]].concat()).is_err());
}

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

/// The IDL leaves `VulnerableLogicRefs` out, so its decoder is the client's
/// own: it reads back the list the program stores.
#[test]
fn the_client_decodes_the_vulnerable_logic_refs_the_program_stores() {
    for count in [0, 1, 5] {
        let logic_refs: Vec<[u8; 32]> = (0..count as u8).map(|i| [i; 32]).collect();
        let mut data = Vec::new();
        VulnerableLogicRefs {
            logic_refs: logic_refs.clone(),
        }
        .try_serialize(&mut data)
        .unwrap();
        let decoded = client::decode_vulnerable_logic_refs(&data);
        println!("{count} refs: {data:02x?} decodes to {decoded:?}");
        assert_eq!(decoded, Ok(logic_refs), "{count} refs");
    }
}

/// The generated `MigrationRecord` reads the record the program writes.
#[test]
fn the_generated_migration_record_reads_the_programs() {
    let mut data = Vec::new();
    MigrationRecord {
        migrated: true,
        bump: 251,
    }
    .try_serialize(&mut data)
    .unwrap();
    let decoded = generated::accounts::MigrationRecord::try_deserialize(&mut data.as_slice())
        .expect("the generated MigrationRecord reads the program's");
    assert!(decoded.migrated);
    assert_eq!(decoded.bump, 251);
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
