//! The emergency-migrating build's instructions and its migration segment,
//! as the EVM bindings' `emergency_migrating_erc20_forwarder` module binds
//! the draft. Its wire format (`MigrateInput`, `OP_MIGRATE`) is in
//! [`input`](crate::input), which compiles without the Solana SDK. The
//! instructions are built over the generated `client::accounts` and
//! `client::args` of `generated::emergency_migrating_spl_token_forwarder`.

use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;

use anoma_pa_solana_client::{
    derive_event_authority_pda, derive_nullifier_pda, derive_pa_state_pda, derive_root_marker_pda,
};

use crate::forwarder::{forwarder_segment_head, instruction};
use crate::generated::emergency_migrating_spl_token_forwarder::client::{accounts, args};
use crate::generated::emergency_migrating_spl_token_forwarder::constants::{
    MIGRATION_RECORD_SEED, VULNERABLE_LOGIC_REFS_SEED,
};
use crate::input::MigrateEntry;
use crate::pda::derive_forwarder_config_pda;

/// Derive the vulnerable logic refs' account. Seed:
/// `[VULNERABLE_LOGIC_REFS_SEED]`.
pub fn derive_vulnerable_logic_refs_pda(forwarder_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VULNERABLE_LOGIC_REFS_SEED], forwarder_program)
}

/// Derive the migration record of the resource with `nullifier`. Seed:
/// `[MIGRATION_RECORD_SEED, nullifier]`.
pub fn derive_migration_record_pda(
    forwarder_program: &Pubkey,
    nullifier: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[MIGRATION_RECORD_SEED, nullifier], forwarder_program)
}

/// Build the emergency-migrating build's `reinitialize`: after upgrading the
/// forwarder to that build, the owner lists `vulnerable_logic_refs`, which
/// the adapter `protocol_adapter` denies on both denylists, and sets the
/// forwarder's logic ref to `new_logic_ref`. The owner pays for the listed
/// refs.
pub fn reinitialize_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    protocol_adapter: &Pubkey,
    new_logic_ref: [u8; 32],
    vulnerable_logic_refs: &[[u8; 32]],
) -> Instruction {
    instruction(
        forwarder_program,
        accounts::Reinitialize {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            vulnerable_logic_refs: derive_vulnerable_logic_refs_pda(forwarder_program).0,
            pa_state: derive_pa_state_pda(protocol_adapter).0,
            system_program: system_program::id(),
            event_authority: derive_event_authority_pda(forwarder_program).0,
            program: *forwarder_program,
        },
        args::Reinitialize {
            new_logic_ref,
            vulnerable_logic_refs: vulnerable_logic_refs.to_vec(),
        },
    )
}

/// Build the permissionless `init_migration_record`, which creates the
/// migration record of the resource with `nullifier`, `payer` funding the
/// rent, and does nothing when it exists. A migration needs every entry's
/// record, so its settlement carries this before the settle instruction.
pub fn init_migration_record_ix(
    forwarder_program: &Pubkey,
    payer: &Pubkey,
    nullifier: &[u8; 32],
) -> Instruction {
    instruction(
        forwarder_program,
        accounts::InitMigrationRecord {
            payer: *payer,
            migration_record: derive_migration_record_pda(forwarder_program, nullifier).0,
            system_program: system_program::id(),
        },
        args::InitMigrationRecord {
            nullifier: *nullifier,
        },
    )
}

/// Build the migration forwarder CPI segment for `entries`, through the
/// adapter `protocol_adapter`: the segment head, the adapter's state and
/// the vulnerable logic refs, then for each entry the adapter's marker of
/// its root, the adapter's marker of its nullifier and its migration record,
/// which must already exist (`init_migration_record_ix`).
pub fn build_migrate_forwarder_accounts(
    forwarder_program: &Pubkey,
    protocol_adapter: &Pubkey,
    entries: &[MigrateEntry],
) -> Vec<AccountMeta> {
    let (pa_state, _) = derive_pa_state_pda(protocol_adapter);
    let mut accounts = forwarder_segment_head(forwarder_program).to_vec();
    accounts.extend([
        AccountMeta::new_readonly(pa_state, false),
        AccountMeta::new_readonly(derive_vulnerable_logic_refs_pda(forwarder_program).0, false),
    ]);
    for entry in entries {
        accounts.extend([
            AccountMeta::new_readonly(
                derive_root_marker_pda(protocol_adapter, &pa_state, &entry.commitment_tree_root).0,
                false,
            ),
            AccountMeta::new_readonly(
                derive_nullifier_pda(protocol_adapter, &pa_state, &entry.nullifier).0,
                false,
            ),
            AccountMeta::new(
                derive_migration_record_pda(forwarder_program, &entry.nullifier).0,
                false,
            ),
        ]);
    }
    accounts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::forwarder_migrate_num_accounts;
    use crate::forwarder::tests::{
        assert_event_accounts, assert_is_expected, assert_matches_the_idl, key,
    };

    /// Each instruction `the_builders_write_the_expected_instructions` builds,
    /// in the form of `forwarder`'s `EXPECTED`.
    const EXPECTED: &str = "\
== emergency_migrating_reinitialize
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 87ffcda1dfa25e8903030303030303030303030303030303030303030303030303030303030303030200000005050505050505050505050505050505050505050505050505050505050505050606060606060606060606060606060606060606060606060606060606060606
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false true
acct G6rYRax9Sm5wfnzuQfxYu4AKZxhhmUq4ifW9YgRgwo6W false true
acct 7ZjH1XmCbc4CjMghfx6A7vnCv9cgwbDyk1uUULs72w5N false false
acct 11111111111111111111111111111111 false false
acct BaNhKDjFJTKQhS1To1XK8eApMSYBoMUNky6wKrf4RX5N false false
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false false
== init_migration_record
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 0b2279b9f727d9cc0505050505050505050505050505050505050505050505050505050505050505
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct 8tjz1v1PzVkq3gpyV82ADm9cr9C9Q9jHszpsqsgQXKDp false true
acct 11111111111111111111111111111111 false false
";

    #[test]
    fn the_builders_write_the_expected_instructions() {
        let p = key(9);
        assert_is_expected(
            EXPECTED,
            "emergency_migrating_reinitialize",
            &reinitialize_ix(&p, &key(1), &key(2), [3; 32], &[[5; 32], [6; 32]]),
        );
        assert_is_expected(
            EXPECTED,
            "init_migration_record",
            &init_migration_record_ix(&p, &key(1), &[5; 32]),
        );
    }

    #[test]
    fn reinitialize_matches_the_idl() {
        let forwarder = crate::generated::emergency_migrating_spl_token_forwarder::ID;
        let (owner, adapter) = (Pubkey::new_unique(), Pubkey::new_unique());
        let ix = reinitialize_ix(&forwarder, &owner, &adapter, [9; 32], &[[1; 32], [2; 32]]);
        let args = [
            [9u8; 32].as_slice(),
            &2u32.to_le_bytes(),
            &[1; 32],
            &[2; 32],
        ]
        .concat();
        let idl = crate::idl::emergency_migrating_idl();
        let by_name = assert_matches_the_idl(&idl, &ix, "reinitialize", &args);
        assert_eq!(by_name("authority"), owner);
        assert_eq!(by_name("pa_state"), derive_pa_state_pda(&adapter).0);
        assert_event_accounts(&by_name, &forwarder);
        assert_eq!(ix.accounts.len(), 7);
    }

    #[test]
    fn init_migration_record_matches_the_idl() {
        let forwarder = crate::generated::emergency_migrating_spl_token_forwarder::ID;
        let payer = Pubkey::new_unique();
        let ix = init_migration_record_ix(&forwarder, &payer, &[5; 32]);
        let idl = crate::idl::emergency_migrating_idl();
        let by_name = assert_matches_the_idl(&idl, &ix, "init_migration_record", &[5; 32]);
        assert_eq!(by_name("payer"), payer);
        assert_eq!(
            by_name("migration_record"),
            derive_migration_record_pda(&forwarder, &[5; 32]).0
        );
        assert_eq!(ix.accounts.len(), 3);
    }

    #[test]
    fn the_migration_segment_has_the_size_a_call_commits() {
        let (forwarder, adapter) = (Pubkey::new_unique(), Pubkey::new_unique());
        let entry = MigrateEntry {
            nullifier: [1; 32],
            commitment_tree_root: [2; 32],
            vulnerable_logic_ref: [3; 32],
            forwarder: forwarder.to_bytes(),
        };
        for entries in [1, 4] {
            let segment = build_migrate_forwarder_accounts(
                &forwarder,
                &adapter,
                &vec![entry.clone(); entries],
            );
            assert_eq!(
                Some(segment.len() as u8),
                forwarder_migrate_num_accounts(entries),
                "{entries} entries"
            );
        }
        let (pa_state, _) = derive_pa_state_pda(&adapter);
        let segment = build_migrate_forwarder_accounts(&forwarder, &adapter, &[entry]);
        assert_eq!(
            segment[5..],
            [
                AccountMeta::new_readonly(pa_state, false),
                AccountMeta::new_readonly(derive_vulnerable_logic_refs_pda(&forwarder).0, false),
                AccountMeta::new_readonly(
                    derive_root_marker_pda(&adapter, &pa_state, &[2; 32]).0,
                    false
                ),
                AccountMeta::new_readonly(
                    derive_nullifier_pda(&adapter, &pa_state, &[1; 32]).0,
                    false
                ),
                AccountMeta::new(derive_migration_record_pda(&forwarder, &[1; 32]).0, false),
            ]
        );
        assert_eq!(
            forwarder_migrate_num_accounts(83),
            None,
            "7 + 3 * 83 exceeds a u8"
        );
    }
}
