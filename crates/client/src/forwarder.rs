//! Builders for the forwarder's CPI account segments inside a settle
//! transaction's `remaining_accounts`, and for the forwarder's own
//! `init_nonce_bitmap` instruction. Ordering is owned by the forwarder program;
//! integrators must use these builders rather than hand-rolling the slice.

use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::{bpf_loader_upgradeable, system_program, sysvar};

use anoma_pa_solana_client::{
    anchor_instruction_disc, derive_event_authority_pda, derive_program_data_address,
    derive_upgrade_authority_pda,
};

use crate::constants::{FORWARDER_UNWRAP_NUM_ACCOUNTS, FORWARDER_WRAP_NUM_ACCOUNTS};
use crate::pda::{
    derive_associated_token_address, derive_forwarder_config_pda,
    derive_forwarder_escrow_authority, derive_nonce_bitmap_pda,
};

/// Nonces per nonce-bitmap account. The bitmap for `nonce` is the one for
/// word `nonce / NONCES_PER_WORD`.
pub const NONCES_PER_WORD: u64 = 256;

/// The nonce-bitmap word a wrap nonce falls in.
pub fn nonce_word_index(nonce: u64) -> u64 {
    nonce / NONCES_PER_WORD
}

/// Build the forwarder's `initialize`: `authority`, the program's upgrade
/// authority, binds the forwarder to `protocol_adapter` and `logic_ref` with
/// its emergency committee and owner, and hands the upgrade authority to the
/// program.
pub fn initialize_ix(
    forwarder_program: &Pubkey,
    authority: &Pubkey,
    protocol_adapter: &Pubkey,
    logic_ref: [u8; 32],
    emergency_committee: &Pubkey,
    initial_owner: &Pubkey,
) -> Instruction {
    let mut data = anchor_instruction_disc("initialize").to_vec();
    data.extend_from_slice(protocol_adapter.as_ref());
    data.extend_from_slice(&logic_ref);
    data.extend_from_slice(emergency_committee.as_ref());
    data.extend_from_slice(initial_owner.as_ref());
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(derive_forwarder_config_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(*forwarder_program, false),
            AccountMeta::new(derive_program_data_address(forwarder_program), false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(derive_upgrade_authority_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(bpf_loader_upgradeable::id(), false),
        ],
        data,
    }
}

/// Build the forwarder's permissionless `init_nonce_bitmap` instruction, which
/// creates `user`'s bitmap for `word_index` with `payer` funding the rent.
///
/// A wrap needs the bitmap for its nonce's word to exist, so a settlement
/// whose word has no bitmap yet (the account at
/// `derive_nonce_bitmap_pda(forwarder_program, user, word_index)` is absent)
/// carries this instruction before the settle instruction. It fits in the
/// settlement transaction after the ed25519 instruction.
pub fn init_nonce_bitmap_ix(
    forwarder_program: &Pubkey,
    payer: &Pubkey,
    user: &Pubkey,
    word_index: u64,
) -> Instruction {
    let (nonce_bitmap_pda, _) = derive_nonce_bitmap_pda(forwarder_program, user, word_index);
    let mut data = Vec::with_capacity(8 + 32 + 8);
    data.extend_from_slice(&anchor_instruction_disc("init_nonce_bitmap"));
    data.extend_from_slice(user.as_ref());
    data.extend_from_slice(&word_index.to_le_bytes());
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(nonce_bitmap_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// The head of every forwarder CPI segment: the forwarder (segment marker),
/// its config, the instructions sysvar, then the forwarder's event authority
/// and the forwarder again, which its CPI events need.
fn forwarder_segment_head(forwarder_program: &Pubkey) -> [AccountMeta; 5] {
    [
        AccountMeta::new_readonly(*forwarder_program, false),
        AccountMeta::new_readonly(derive_forwarder_config_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(sysvar::instructions::id(), false),
        AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(*forwarder_program, false),
    ]
}

/// Build the wrap forwarder CPI segment.
///
/// Order: `[forwarder_program, config, ix_sysvar, event_authority,
/// forwarder_program, user_ata, escrow_ata, escrow_authority,
/// nonce_bitmap_pda, token_program]`.
pub fn build_wrap_forwarder_accounts(
    forwarder_program: &Pubkey,
    user: &Pubkey,
    token_mint: &Pubkey,
    nonce: u64,
) -> Vec<AccountMeta> {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    let user_ata = derive_associated_token_address(user, token_mint);
    let escrow_ata = derive_associated_token_address(&escrow_authority, token_mint);
    let (nonce_bitmap_pda, _) =
        derive_nonce_bitmap_pda(forwarder_program, user, nonce_word_index(nonce));

    let mut accounts = forwarder_segment_head(forwarder_program).to_vec();
    accounts.extend([
        AccountMeta::new(user_ata, false),                  // user ATA
        AccountMeta::new(escrow_ata, false),                // escrow ATA
        AccountMeta::new_readonly(escrow_authority, false), // escrow authority
        AccountMeta::new(nonce_bitmap_pda, false),          // nonce bitmap
        AccountMeta::new_readonly(spl_token_interface::id(), false), // token program
    ]);
    debug_assert_eq!(accounts.len(), FORWARDER_WRAP_NUM_ACCOUNTS as usize);
    accounts
}

/// Build the unwrap forwarder CPI segment.
///
/// Order: `[forwarder_program, config, ix_sysvar, event_authority,
/// forwarder_program, escrow_ata, recipient_ata, escrow_authority,
/// token_program]`.
pub fn build_unwrap_forwarder_accounts(
    forwarder_program: &Pubkey,
    recipient: &Pubkey,
    token_mint: &Pubkey,
) -> Vec<AccountMeta> {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    let escrow_ata = derive_associated_token_address(&escrow_authority, token_mint);
    let recipient_ata = derive_associated_token_address(recipient, token_mint);

    let mut accounts = forwarder_segment_head(forwarder_program).to_vec();
    accounts.extend([
        AccountMeta::new(escrow_ata, false),                // escrow ATA
        AccountMeta::new(recipient_ata, false),             // recipient ATA
        AccountMeta::new_readonly(escrow_authority, false), // escrow authority
        AccountMeta::new_readonly(spl_token_interface::id(), false), // token program
    ]);
    debug_assert_eq!(accounts.len(), FORWARDER_UNWRAP_NUM_ACCOUNTS as usize);
    accounts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn canonical_spl_token_program_id() -> Pubkey {
        Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()
    }

    fn keys(accounts: &[AccountMeta]) -> Vec<Pubkey> {
        accounts.iter().map(|a| a.pubkey).collect()
    }

    #[test]
    fn wrap_segment_is_the_forwarder_layout() {
        let forwarder = Pubkey::new_unique();
        let user = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let nonce = 300;
        let accs = build_wrap_forwarder_accounts(&forwarder, &user, &mint, nonce);
        assert_eq!(accs.len(), FORWARDER_WRAP_NUM_ACCOUNTS as usize);

        let escrow_authority = derive_forwarder_escrow_authority(&forwarder).0;
        assert_eq!(
            keys(&accs),
            vec![
                forwarder,
                derive_forwarder_config_pda(&forwarder).0,
                sysvar::instructions::id(),
                derive_event_authority_pda(&forwarder).0,
                forwarder,
                derive_associated_token_address(&user, &mint),
                derive_associated_token_address(&escrow_authority, &mint),
                escrow_authority,
                derive_nonce_bitmap_pda(&forwarder, &user, 1).0, // nonce 300 is in word 1
                canonical_spl_token_program_id(),
            ]
        );
        let writable: Vec<bool> = accs.iter().map(|a| a.is_writable).collect();
        assert_eq!(
            writable,
            [false, false, false, false, false, true, true, false, true, false],
            "the user ATA, escrow ATA and nonce bitmap are written"
        );
        assert!(accs.iter().all(|a| !a.is_signer));
    }

    #[test]
    fn unwrap_segment_is_the_forwarder_layout() {
        let forwarder = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let accs = build_unwrap_forwarder_accounts(&forwarder, &recipient, &mint);
        assert_eq!(accs.len(), FORWARDER_UNWRAP_NUM_ACCOUNTS as usize);

        let escrow_authority = derive_forwarder_escrow_authority(&forwarder).0;
        assert_eq!(
            keys(&accs),
            vec![
                forwarder,
                derive_forwarder_config_pda(&forwarder).0,
                sysvar::instructions::id(),
                derive_event_authority_pda(&forwarder).0,
                forwarder,
                derive_associated_token_address(&escrow_authority, &mint),
                derive_associated_token_address(&recipient, &mint),
                escrow_authority,
                canonical_spl_token_program_id(),
            ]
        );
        let writable: Vec<bool> = accs.iter().map(|a| a.is_writable).collect();
        assert_eq!(
            writable,
            [false, false, false, false, false, true, true, false, false],
            "the escrow ATA and recipient ATA are written"
        );
        assert!(accs.iter().all(|a| !a.is_signer));
    }

    #[test]
    fn nonce_word_index_covers_256_nonces_per_word() {
        assert_eq!(nonce_word_index(0), 0);
        assert_eq!(nonce_word_index(255), 0);
        assert_eq!(nonce_word_index(256), 1);
        assert_eq!(nonce_word_index(u64::MAX), u64::MAX / 256);
    }

    #[test]
    fn init_nonce_bitmap_ix_matches_the_forwarder_idl() {
        let forwarder = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let user = Pubkey::new_unique();
        let ix = init_nonce_bitmap_ix(&forwarder, &payer, &user, 7);

        assert_eq!(ix.program_id, forwarder);
        // Discriminator from idl/spl_token_forwarder.json, then the two args.
        assert_eq!(&ix.data[..8], &[214, 13, 125, 121, 72, 220, 241, 42]);
        assert_eq!(&ix.data[8..40], user.as_ref());
        assert_eq!(&ix.data[40..], &7u64.to_le_bytes());

        let expected_bitmap = derive_nonce_bitmap_pda(&forwarder, &user, 7).0;
        assert_eq!(
            ix.accounts,
            vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(expected_bitmap, false),
                AccountMeta::new_readonly(system_program::id(), false),
            ]
        );
    }

    #[test]
    fn initialize_matches_the_forwarders_idl() {
        let idl: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/idl/spl_token_forwarder.json"
        )))
        .unwrap();
        let spec = idl["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ix| ix["name"] == "initialize")
            .unwrap();
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
        let (authority, adapter, committee, owner) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let ix = initialize_ix(
            &forwarder, &authority, &adapter, [7; 32], &committee, &owner,
        );

        let mut data: Vec<u8> = serde_json::from_value(spec["discriminator"].clone()).unwrap();
        data.extend(adapter.to_bytes());
        data.extend([7; 32]);
        data.extend(committee.to_bytes());
        data.extend(owner.to_bytes());
        assert_eq!(
            ix.data, data,
            "discriminator, then adapter, logic ref, committee, owner"
        );

        // Each account's flags, its fixed address, or its PDA from the IDL's
        // own constant seeds.
        let accounts = spec["accounts"].as_array().unwrap();
        assert_eq!(ix.accounts.len(), accounts.len());
        for (meta, account) in ix.accounts.iter().zip(accounts) {
            let name = account["name"].as_str().unwrap();
            assert_eq!(
                meta.is_writable,
                account["writable"] == true,
                "{name} writable"
            );
            assert_eq!(meta.is_signer, account["signer"] == true, "{name} signer");
            if let Some(address) = account["address"].as_str() {
                assert_eq!(meta.pubkey.to_string(), address, "{name} address");
            }
            if let Some(seeds) = account["pda"]["seeds"].as_array() {
                let seeds: Vec<Vec<u8>> = seeds
                    .iter()
                    .map(|s| serde_json::from_value(s["value"].clone()).unwrap())
                    .collect();
                let seeds: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
                assert_eq!(
                    meta.pubkey,
                    Pubkey::find_program_address(&seeds, &forwarder).0,
                    "{name} PDA"
                );
            }
        }
        assert_eq!(ix.accounts[0].pubkey, authority);
    }
}
