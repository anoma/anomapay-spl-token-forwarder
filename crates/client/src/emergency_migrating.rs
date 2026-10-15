//! The emergency-migrating build's instructions and its migration segment,
//! as the EVM bindings' `emergency_migrating_erc20_forwarder` module binds
//! the draft. Its wire format (`MigrateInput`, `OP_MIGRATE`) is in
//! [`input`](crate::input), which compiles without the Solana SDK.

use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::system_program;

use anoma_pa_solana_client::{
    anchor_instruction_disc, derive_event_authority_pda, derive_nullifier_pda, derive_pa_state_pda,
    derive_root_marker_pda,
};

use crate::forwarder::forwarder_segment_head;
use crate::input::MigrateEntry;
use crate::pda::derive_forwarder_config_pda;

/// Derive the vulnerable logic refs' account. Seed:
/// `["vulnerable_logic_refs"]`.
pub fn derive_vulnerable_logic_refs_pda(forwarder_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vulnerable_logic_refs"], forwarder_program)
}

/// Derive the migration record of the resource with `nullifier`. Seed:
/// `["migration_record", nullifier]`.
pub fn derive_migration_record_pda(
    forwarder_program: &Pubkey,
    nullifier: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"migration_record", nullifier], forwarder_program)
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
    let mut data = anchor_instruction_disc("reinitialize").to_vec();
    data.extend_from_slice(&new_logic_ref);
    let count =
        u32::try_from(vulnerable_logic_refs.len()).expect("fewer than 2^32 vulnerable logic refs");
    data.extend_from_slice(&count.to_le_bytes());
    vulnerable_logic_refs
        .iter()
        .for_each(|logic_ref| data.extend_from_slice(logic_ref));
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new(*owner, true),
            AccountMeta::new(derive_forwarder_config_pda(forwarder_program).0, false),
            AccountMeta::new(derive_vulnerable_logic_refs_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(derive_pa_state_pda(protocol_adapter).0, false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(*forwarder_program, false),
        ],
        data,
    }
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
    let mut data = anchor_instruction_disc("init_migration_record").to_vec();
    data.extend_from_slice(nullifier);
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(
                derive_migration_record_pda(forwarder_program, nullifier).0,
                false,
            ),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
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
    use crate::forwarder::tests::{assert_event_accounts, assert_matches_the_idl};

    #[test]
    fn reinitialize_matches_the_idl() {
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
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
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
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
