//! This crate against the program's emergency-migrating build: the migrate
//! input it encodes is the one the program parses, and the accounts and
//! addresses it decodes and derives are the program's.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{AccountDeserialize, AccountSerialize, AnchorDeserialize};
use anomapay_spl_token_forwarder_client as client;
use client::generated::emergency_migrating_spl_token_forwarder as generated;
use spl_token_forwarder::emergency_migrating::{
    MigrateInput, MigrationRecord, VulnerableLogicRefs, MIGRATION_RECORD_SEED, OP_MIGRATE,
};

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

/// The client derives a resource's migration record address from the
/// program's own seed. The IDL cannot check it: its seeds name the arg
/// `nullifier`, which it lists as the handler's `_nullifier`.
#[test]
fn the_clients_migration_record_address_is_the_programs() {
    for nullifier in [[0u8; 32], [3; 32], [0xff; 32]] {
        let client = client::emergency_migrating::derive_migration_record_pda(
            &spl_token_forwarder::ID,
            &nullifier,
        );
        let program = Pubkey::find_program_address(
            &[MIGRATION_RECORD_SEED, &nullifier],
            &spl_token_forwarder::ID,
        );
        println!("{nullifier:02x?}: {client:?}");
        assert_eq!(client, program, "{nullifier:02x?}");
    }
}
