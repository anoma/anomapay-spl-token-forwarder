//! Builders for the forwarder's CPI account segments inside a settle
//! transaction's `remaining_accounts`, and for the forwarder's own
//! `init_nonce_bitmap` instruction. Ordering is owned by the forwarder program;
//! integrators must use these builders rather than hand-rolling the slice.

use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::{bpf_loader_upgradeable, system_program, sysvar};
use spl_associated_token_account_interface::address::get_associated_token_address;

use anoma_pa_solana_client::{
    anchor_instruction_disc, derive_event_authority_pda, derive_program_data_address,
    derive_upgrade_authority_pda,
};

use crate::constants::NONCES_PER_WORD;
use crate::input::UnwrapInput;
use crate::pda::{
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, derive_nonce_bitmap_pda,
};

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

/// The accounts of an owner-only instruction (`onlyOwner`): the owner, the
/// config, then the event authority and the forwarder, which its events need.
fn owner_only_accounts(forwarder_program: &Pubkey, owner: &Pubkey) -> Vec<AccountMeta> {
    vec![
        AccountMeta::new_readonly(*owner, true),
        AccountMeta::new(derive_forwarder_config_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(*forwarder_program, false),
    ]
}

/// Build the forwarder's `reinitialize`: the owner moves the forwarder to the
/// resource logic `logic_ref`.
pub fn reinitialize_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    logic_ref: [u8; 32],
) -> Instruction {
    let mut data = anchor_instruction_disc("reinitialize").to_vec();
    data.extend_from_slice(&logic_ref);
    Instruction {
        program_id: *forwarder_program,
        accounts: owner_only_accounts(forwarder_program, owner),
        data,
    }
}

/// Build the forwarder's `transfer_ownership` to `new_owner`.
pub fn transfer_ownership_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    new_owner: &Pubkey,
) -> Instruction {
    let mut data = anchor_instruction_disc("transfer_ownership").to_vec();
    data.extend_from_slice(new_owner.as_ref());
    Instruction {
        program_id: *forwarder_program,
        accounts: owner_only_accounts(forwarder_program, owner),
        data,
    }
}

/// Build the forwarder's `renounce_ownership`: it is left with no owner.
pub fn renounce_ownership_ix(forwarder_program: &Pubkey, owner: &Pubkey) -> Instruction {
    Instruction {
        program_id: *forwarder_program,
        accounts: owner_only_accounts(forwarder_program, owner),
        data: anchor_instruction_disc("renounce_ownership").to_vec(),
    }
}

/// The compute-unit limit an upgrade asks for: the most a transaction may
/// have (Agave's `MAX_COMPUTE_UNIT_LIMIT`). The upgrade hashes the whole
/// buffer, so its cost grows with the program's code.
pub const UPGRADE_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

/// Build the forwarder's `upgrade`, after the compute budget it needs: the
/// owner replaces the program's code with the loader buffer `buffer`, whose
/// authority is the owner; the buffer's rent goes to `spill`.
pub fn upgrade_ixs(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    buffer: &Pubkey,
    spill: &Pubkey,
) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(UPGRADE_COMPUTE_UNIT_LIMIT),
        Instruction {
            program_id: *forwarder_program,
            accounts: vec![
                AccountMeta::new_readonly(*owner, true),
                AccountMeta::new_readonly(derive_forwarder_config_pda(forwarder_program).0, false),
                AccountMeta::new(derive_program_data_address(forwarder_program), false),
                AccountMeta::new(*forwarder_program, false),
                AccountMeta::new(*buffer, false),
                AccountMeta::new(*spill, false),
                AccountMeta::new_readonly(derive_upgrade_authority_pda(forwarder_program).0, false),
                AccountMeta::new_readonly(sysvar::rent::id(), false),
                AccountMeta::new_readonly(sysvar::clock::id(), false),
                AccountMeta::new_readonly(bpf_loader_upgradeable::id(), false),
                AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
                AccountMeta::new_readonly(*forwarder_program, false),
            ],
            data: anchor_instruction_disc("upgrade").to_vec(),
        },
    ]
}

/// Build the forwarder's `version`, whose return data is the program's
/// version string.
pub fn version_ix(forwarder_program: &Pubkey) -> Instruction {
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![],
        data: anchor_instruction_disc("version").to_vec(),
    }
}

/// Build the forwarder's `set_emergency_caller`: the emergency committee
/// names the one emergency caller, once, while the adapter whose state is
/// `pa_state` is paused.
pub fn set_emergency_caller_ix(
    forwarder_program: &Pubkey,
    committee: &Pubkey,
    pa_state: &Pubkey,
    new_emergency_caller: &Pubkey,
) -> Instruction {
    let mut data = anchor_instruction_disc("set_emergency_caller").to_vec();
    data.extend_from_slice(new_emergency_caller.as_ref());
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new_readonly(*committee, true),
            AccountMeta::new(derive_forwarder_config_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(*pa_state, false),
            AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(*forwarder_program, false),
        ],
        data,
    }
}

/// Build the forwarder's `forward_emergency_call`: while the adapter whose
/// state is `pa_state` is paused, the emergency caller releases
/// `withdraw.amount` of `withdraw.token_mint` from escrow to
/// `withdraw.recipient`'s token account.
pub fn forward_emergency_call_ix(
    forwarder_program: &Pubkey,
    caller: &Pubkey,
    pa_state: &Pubkey,
    withdraw: &UnwrapInput,
) -> Instruction {
    let input = [
        withdraw.token_mint.as_slice(),
        &withdraw.amount.to_le_bytes(),
        &withdraw.recipient,
    ]
    .concat();
    let mut data = anchor_instruction_disc("forward_emergency_call").to_vec();
    data.extend_from_slice(&(input.len() as u32).to_le_bytes());
    data.extend_from_slice(&input);
    let mut accounts = vec![
        AccountMeta::new(*caller, true),
        AccountMeta::new_readonly(derive_forwarder_config_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(*pa_state, false),
        AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(*forwarder_program, false),
    ];
    accounts.extend(escrow_release_accounts(
        forwarder_program,
        &Pubkey::new_from_array(withdraw.recipient),
        &Pubkey::new_from_array(withdraw.token_mint),
    ));
    Instruction {
        program_id: *forwarder_program,
        accounts,
        data,
    }
}

/// Build the forwarder's `close_escrow`: while the adapter whose state is
/// `pa_state` is paused, the emergency committee drains `mint`'s escrow to
/// the token account `recipient_ata` and closes it, taking its rent.
pub fn close_escrow_ix(
    forwarder_program: &Pubkey,
    committee: &Pubkey,
    pa_state: &Pubkey,
    mint: &Pubkey,
    recipient_ata: &Pubkey,
) -> Instruction {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new(*committee, true),
            AccountMeta::new_readonly(derive_forwarder_config_pda(forwarder_program).0, false),
            AccountMeta::new(get_associated_token_address(&escrow_authority, mint), false),
            AccountMeta::new_readonly(escrow_authority, false),
            AccountMeta::new(*recipient_ata, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(spl_token_interface::id(), false),
            AccountMeta::new_readonly(*pa_state, false),
        ],
        data: anchor_instruction_disc("close_escrow").to_vec(),
    }
}

/// Build the forwarder's `close_nonce_bitmaps_batch`: while the adapter
/// whose state is `pa_state` is paused, the emergency committee closes the
/// nonce bitmaps `bitmaps`, taking their rent.
pub fn close_nonce_bitmaps_batch_ix(
    forwarder_program: &Pubkey,
    committee: &Pubkey,
    pa_state: &Pubkey,
    bitmaps: &[Pubkey],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*committee, true),
        AccountMeta::new_readonly(derive_forwarder_config_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(*pa_state, false),
    ];
    accounts.extend(
        bitmaps
            .iter()
            .map(|bitmap| AccountMeta::new(*bitmap, false)),
    );
    Instruction {
        program_id: *forwarder_program,
        accounts,
        data: anchor_instruction_disc("close_nonce_bitmaps_batch").to_vec(),
    }
}

/// Build the forwarder's `close_config`: while the adapter whose state is
/// `pa_state` is paused, the emergency committee closes the config, taking
/// its rent. The forwarder serves nothing after it.
pub fn close_config_ix(
    forwarder_program: &Pubkey,
    committee: &Pubkey,
    pa_state: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: *forwarder_program,
        accounts: vec![
            AccountMeta::new(*committee, true),
            AccountMeta::new(derive_forwarder_config_pda(forwarder_program).0, false),
            AccountMeta::new_readonly(*pa_state, false),
        ],
        data: anchor_instruction_disc("close_config").to_vec(),
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

/// The accounts of a release from `token_mint`'s escrow to `recipient`'s
/// token account, in the order the forwarder reads them: the escrow's token
/// account, the recipient's, the escrow authority and the token program.
fn escrow_release_accounts(
    forwarder_program: &Pubkey,
    recipient: &Pubkey,
    token_mint: &Pubkey,
) -> [AccountMeta; 4] {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    [
        AccountMeta::new(
            get_associated_token_address(&escrow_authority, token_mint),
            false,
        ),
        AccountMeta::new(get_associated_token_address(recipient, token_mint), false),
        AccountMeta::new_readonly(escrow_authority, false),
        AccountMeta::new_readonly(spl_token_interface::id(), false),
    ]
}

/// Build the wrap forwarder CPI segment: `[forwarder_program, config,
/// ix_sysvar, event_authority, forwarder_program, user_ata, escrow_ata,
/// escrow_authority, nonce_bitmap_pda, token_program]`. The nonce bitmap must
/// already exist (`init_nonce_bitmap_ix`).
pub fn build_wrap_forwarder_accounts(
    forwarder_program: &Pubkey,
    user: &Pubkey,
    token_mint: &Pubkey,
    nonce: u64,
) -> Vec<AccountMeta> {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    let (nonce_bitmap_pda, _) =
        derive_nonce_bitmap_pda(forwarder_program, user, nonce_word_index(nonce));
    let mut accounts = forwarder_segment_head(forwarder_program).to_vec();
    accounts.extend([
        AccountMeta::new(get_associated_token_address(user, token_mint), false),
        AccountMeta::new(
            get_associated_token_address(&escrow_authority, token_mint),
            false,
        ),
        AccountMeta::new_readonly(escrow_authority, false),
        AccountMeta::new(nonce_bitmap_pda, false),
        AccountMeta::new_readonly(spl_token_interface::id(), false),
    ]);
    accounts
}

/// Build the unwrap forwarder CPI segment: `[forwarder_program, config,
/// ix_sysvar, event_authority, forwarder_program, escrow_ata, recipient_ata,
/// escrow_authority, token_program]`.
pub fn build_unwrap_forwarder_accounts(
    forwarder_program: &Pubkey,
    recipient: &Pubkey,
    token_mint: &Pubkey,
) -> Vec<AccountMeta> {
    let mut accounts = forwarder_segment_head(forwarder_program).to_vec();
    accounts.extend(escrow_release_accounts(
        forwarder_program,
        recipient,
        token_mint,
    ));
    accounts
}

/// The accounts every settlement that calls the forwarder carries for it and
/// that are the same for every call: the ones a deployment's settlement
/// lookup table holds for it. Each mint in `mints` adds its escrow's token
/// account; the user's and the recipient's token accounts and the user's
/// nonce bitmap differ per settlement.
pub fn forwarder_settlement_lookup_keys(
    forwarder_program: &Pubkey,
    mints: &[Pubkey],
) -> Vec<Pubkey> {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    let mut keys = vec![
        *forwarder_program,
        derive_forwarder_config_pda(forwarder_program).0,
        sysvar::instructions::id(),
        derive_event_authority_pda(forwarder_program).0,
        escrow_authority,
        spl_token_interface::id(),
    ];
    keys.extend(
        mints
            .iter()
            .map(|mint| get_associated_token_address(&escrow_authority, mint)),
    );
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{FORWARDER_UNWRAP_NUM_ACCOUNTS, FORWARDER_WRAP_NUM_ACCOUNTS};
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
                get_associated_token_address(&user, &mint),
                get_associated_token_address(&escrow_authority, &mint),
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
                get_associated_token_address(&escrow_authority, &mint),
                get_associated_token_address(&recipient, &mint),
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
    fn the_lookup_keys_are_the_segments_accounts_no_user_or_recipient_changes() {
        let forwarder = Pubkey::new_unique();
        let (user, recipient) = (Pubkey::new_unique(), Pubkey::new_unique());
        let mints = [Pubkey::new_unique(), Pubkey::new_unique()];
        let lookup = forwarder_settlement_lookup_keys(&forwarder, &mints);
        for mint in &mints {
            let per_settlement = [
                get_associated_token_address(&user, mint),
                derive_nonce_bitmap_pda(&forwarder, &user, 0).0,
                get_associated_token_address(&recipient, mint),
            ];
            let segments = keys(&build_wrap_forwarder_accounts(&forwarder, &user, mint, 0))
                .into_iter()
                .chain(keys(&build_unwrap_forwarder_accounts(
                    &forwarder, &recipient, mint,
                )));
            for key in segments {
                assert_eq!(
                    lookup.contains(&key),
                    !per_settlement.contains(&key),
                    "{key} is in the lookup keys {lookup:?} exactly when no settlement changes it"
                );
            }
        }
        // The six accounts every call shares and one escrow per mint: every
        // fixed segment account once, nothing else.
        assert_eq!(lookup.len(), 6 + mints.len(), "{lookup:?}");
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

    /// Checks `ix` against the forwarder IDL's instruction `name`: the data is
    /// its discriminator then `args`, and each account has the IDL's flags,
    /// its fixed address, or its PDA from the IDL's own constant seeds.
    /// Returns the address `ix` passes for each IDL account name.
    fn assert_matches_the_forwarders_idl(
        ix: &Instruction,
        name: &str,
        args: &[u8],
    ) -> impl Fn(&str) -> Pubkey {
        let idl = crate::idl::idl();
        let spec = idl["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|spec| spec["name"] == name)
            .unwrap()
            .clone();

        let mut data: Vec<u8> = serde_json::from_value(spec["discriminator"].clone()).unwrap();
        data.extend(args);
        assert_eq!(
            ix.data, data,
            "{name}: the discriminator, then the arguments"
        );

        let accounts = spec["accounts"].as_array().unwrap().clone();
        assert!(
            ix.accounts.len() >= accounts.len(),
            "{name}: {} accounts, the IDL names {}",
            ix.accounts.len(),
            accounts.len()
        );
        for (meta, account) in ix.accounts.iter().zip(&accounts) {
            let account_name = account["name"].as_str().unwrap();
            assert_eq!(
                meta.is_writable,
                account["writable"] == true,
                "{account_name} writable"
            );
            assert_eq!(
                meta.is_signer,
                account["signer"] == true,
                "{account_name} signer"
            );
            if let Some(address) = account["address"].as_str() {
                assert_eq!(meta.pubkey.to_string(), address, "{account_name} address");
            }
            if let Some(seeds) = account["pda"]["seeds"].as_array() {
                let seeds: Vec<Vec<u8>> = seeds
                    .iter()
                    .map(|s| serde_json::from_value(s["value"].clone()).unwrap())
                    .collect();
                let seeds: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
                assert_eq!(
                    meta.pubkey,
                    Pubkey::find_program_address(&seeds, &ix.program_id).0,
                    "{account_name} PDA"
                );
            }
        }
        let addresses = ix
            .accounts
            .iter()
            .map(|meta| meta.pubkey)
            .collect::<Vec<_>>();
        move |name: &str| {
            let i = accounts.iter().position(|a| a["name"] == name).unwrap();
            addresses[i]
        }
    }

    /// The events' accounts every `#[event_cpi]` instruction ends with.
    fn assert_event_accounts(by_name: &impl Fn(&str) -> Pubkey, forwarder: &Pubkey) {
        assert_eq!(
            by_name("event_authority"),
            derive_event_authority_pda(forwarder).0
        );
        assert_eq!(by_name("program"), *forwarder);
    }

    #[test]
    fn initialize_matches_the_forwarders_idl() {
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
        let args = [
            adapter.to_bytes().as_slice(),
            &[7; 32],
            &committee.to_bytes(),
            &owner.to_bytes(),
        ]
        .concat();
        let by_name = assert_matches_the_forwarders_idl(&ix, "initialize", &args);
        assert_eq!(by_name("authority"), authority);
        assert_eq!(ix.accounts.len(), 8);
    }

    #[test]
    fn the_owner_only_instructions_match_the_forwarders_idl() {
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
        let (owner, new_owner) = (Pubkey::new_unique(), Pubkey::new_unique());
        for (ix, name, args) in [
            (
                reinitialize_ix(&forwarder, &owner, [9; 32]),
                "reinitialize",
                vec![9; 32],
            ),
            (
                transfer_ownership_ix(&forwarder, &owner, &new_owner),
                "transfer_ownership",
                new_owner.to_bytes().to_vec(),
            ),
            (
                renounce_ownership_ix(&forwarder, &owner),
                "renounce_ownership",
                vec![],
            ),
        ] {
            let by_name = assert_matches_the_forwarders_idl(&ix, name, &args);
            assert_eq!(by_name("authority"), owner, "{name}");
            assert_event_accounts(&by_name, &forwarder);
            assert_eq!(ix.accounts.len(), 4, "{name}");
        }
    }

    #[test]
    fn upgrade_sets_its_compute_budget_and_matches_the_forwarders_idl() {
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
        let (owner, buffer, spill) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let [budget, ix] = upgrade_ixs(&forwarder, &owner, &buffer, &spill);
        assert_eq!(
            budget,
            ComputeBudgetInstruction::set_compute_unit_limit(UPGRADE_COMPUTE_UNIT_LIMIT)
        );
        let by_name = assert_matches_the_forwarders_idl(&ix, "upgrade", &[]);
        assert_eq!(by_name("authority"), owner);
        assert_eq!(by_name("buffer"), buffer);
        assert_eq!(by_name("spill"), spill);
        assert_event_accounts(&by_name, &forwarder);
        assert_eq!(ix.accounts.len(), 12);
    }

    #[test]
    fn version_matches_the_forwarders_idl() {
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
        let ix = version_ix(&forwarder);
        let _ = assert_matches_the_forwarders_idl(&ix, "version", &[]);
        assert!(ix.accounts.is_empty());
    }

    #[test]
    fn the_committee_instructions_match_the_forwarders_idl() {
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
        let (committee, pa_state, caller, mint, recipient_ata) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );

        let ix = set_emergency_caller_ix(&forwarder, &committee, &pa_state, &caller);
        let by_name =
            assert_matches_the_forwarders_idl(&ix, "set_emergency_caller", caller.as_ref());
        assert_eq!(by_name("committee"), committee);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_event_accounts(&by_name, &forwarder);
        assert_eq!(ix.accounts.len(), 5);

        let ix = close_escrow_ix(&forwarder, &committee, &pa_state, &mint, &recipient_ata);
        let by_name = assert_matches_the_forwarders_idl(&ix, "close_escrow", &[]);
        let escrow_authority = derive_forwarder_escrow_authority(&forwarder).0;
        assert_eq!(by_name("authority"), committee);
        assert_eq!(
            by_name("escrow_ata"),
            get_associated_token_address(&escrow_authority, &mint)
        );
        assert_eq!(by_name("recipient_ata"), recipient_ata);
        assert_eq!(by_name("token_mint"), mint);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_eq!(ix.accounts.len(), 8);

        let ix = close_config_ix(&forwarder, &committee, &pa_state);
        let by_name = assert_matches_the_forwarders_idl(&ix, "close_config", &[]);
        assert_eq!(by_name("authority"), committee);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_eq!(ix.accounts.len(), 3);

        let bitmaps = [Pubkey::new_unique(), Pubkey::new_unique()];
        let ix = close_nonce_bitmaps_batch_ix(&forwarder, &committee, &pa_state, &bitmaps);
        let by_name = assert_matches_the_forwarders_idl(&ix, "close_nonce_bitmaps_batch", &[]);
        assert_eq!(by_name("authority"), committee);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_eq!(
            ix.accounts[3..].to_vec(),
            bitmaps
                .map(|bitmap| AccountMeta::new(bitmap, false))
                .to_vec(),
            "the bitmaps, writable, as remaining accounts"
        );
    }

    #[test]
    fn forward_emergency_call_matches_the_forwarders_idl() {
        let forwarder = crate::program_ids::FORWARDER_PROGRAM_ID;
        let (caller, pa_state, mint, recipient) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let withdraw = UnwrapInput {
            token_mint: mint.to_bytes(),
            amount: 5,
            recipient: recipient.to_bytes(),
        };
        let ix = forward_emergency_call_ix(&forwarder, &caller, &pa_state, &withdraw);
        let operand = [mint.as_ref(), &5u64.to_le_bytes(), recipient.as_ref()].concat();
        let args = [(operand.len() as u32).to_le_bytes().as_slice(), &operand].concat();
        let by_name = assert_matches_the_forwarders_idl(&ix, "forward_emergency_call", &args);
        assert_eq!(by_name("caller"), caller);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_event_accounts(&by_name, &forwarder);

        let escrow_authority = derive_forwarder_escrow_authority(&forwarder).0;
        assert_eq!(
            ix.accounts[5..].to_vec(),
            vec![
                AccountMeta::new(
                    get_associated_token_address(&escrow_authority, &mint),
                    false
                ),
                AccountMeta::new(get_associated_token_address(&recipient, &mint), false),
                AccountMeta::new_readonly(escrow_authority, false),
                AccountMeta::new_readonly(spl_token_interface::id(), false),
            ],
            "the escrow ATA, recipient ATA, escrow authority and token program follow"
        );
    }
}
