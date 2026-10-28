//! Builders for the forwarder's instructions, over the generated
//! `client::accounts` and `client::args` of `generated::spl_token_forwarder`,
//! and for the forwarder's CPI account segments inside a settle
//! transaction's `remaining_accounts`, which no IDL lists. Each builder
//! takes the address the forwarder is deployed at. Ordering is owned by the
//! forwarder program; integrators must use these builders rather than
//! hand-rolling the slice.

use anchor_lang::{InstructionData, ToAccountMetas};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_sdk_ids::{bpf_loader_upgradeable, system_program, sysvar};
use spl_associated_token_account_interface::address::get_associated_token_address;

use anoma_pa_solana_client::{
    derive_event_authority_pda, derive_program_data_address, derive_upgrade_authority_pda,
};

use crate::constants::FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS;
use crate::generated::spl_token_forwarder::client::{accounts, args};
use crate::generated::spl_token_forwarder::constants::NONCES_PER_WORD;
use crate::pda::{
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, derive_nonce_bitmap_pda,
};

/// The instruction of the forwarder at `forwarder_program` with `accounts`
/// and `args`.
pub(crate) fn instruction(
    forwarder_program: &Pubkey,
    accounts: impl ToAccountMetas,
    args: impl InstructionData,
) -> Instruction {
    Instruction {
        program_id: *forwarder_program,
        accounts: accounts.to_account_metas(None),
        data: args.data(),
    }
}

/// The nonce-bitmap word a wrap nonce falls in.
pub fn nonce_word_index(nonce: u64) -> u64 {
    nonce / NONCES_PER_WORD
}

/// Build the forwarder's `initialize`: `authority`, the program's upgrade
/// authority, binds the forwarder to `protocol_adapter` and `logic_ref` with
/// its owner, and hands the upgrade authority to the program.
pub fn initialize_ix(
    forwarder_program: &Pubkey,
    authority: &Pubkey,
    protocol_adapter: &Pubkey,
    logic_ref: [u8; 32],
    initial_owner: &Pubkey,
) -> Instruction {
    instruction(
        forwarder_program,
        accounts::Initialize {
            authority: *authority,
            config: derive_forwarder_config_pda(forwarder_program).0,
            program: *forwarder_program,
            program_data: derive_program_data_address(forwarder_program),
            system_program: system_program::id(),
            event_authority: derive_event_authority_pda(forwarder_program).0,
            upgrade_authority: derive_upgrade_authority_pda(forwarder_program).0,
            bpf_loader_upgradeable: bpf_loader_upgradeable::id(),
        },
        args::Initialize {
            protocol_adapter: *protocol_adapter,
            logic_ref,
            initial_owner: *initial_owner,
        },
    )
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
    instruction(
        forwarder_program,
        accounts::InitNonceBitmap {
            payer: *payer,
            nonce_bitmap: derive_nonce_bitmap_pda(forwarder_program, user, word_index).0,
            system_program: system_program::id(),
        },
        args::InitNonceBitmap {
            _user: *user,
            _word_index: word_index,
        },
    )
}

/// Build the forwarder's `reinitialize`: the owner moves the forwarder to the
/// resource logic `logic_ref`.
pub fn reinitialize_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    logic_ref: [u8; 32],
) -> Instruction {
    instruction(
        forwarder_program,
        accounts::Reinitialize {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            event_authority: derive_event_authority_pda(forwarder_program).0,
            program: *forwarder_program,
        },
        args::Reinitialize { logic_ref },
    )
}

/// Build the forwarder's `transfer_ownership` to `new_owner`.
pub fn transfer_ownership_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    new_owner: &Pubkey,
) -> Instruction {
    instruction(
        forwarder_program,
        accounts::TransferOwnership {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            event_authority: derive_event_authority_pda(forwarder_program).0,
            program: *forwarder_program,
        },
        args::TransferOwnership {
            new_owner: *new_owner,
        },
    )
}

/// Build the forwarder's `renounce_ownership`: it is left with no owner.
pub fn renounce_ownership_ix(forwarder_program: &Pubkey, owner: &Pubkey) -> Instruction {
    instruction(
        forwarder_program,
        accounts::RenounceOwnership {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            event_authority: derive_event_authority_pda(forwarder_program).0,
            program: *forwarder_program,
        },
        args::RenounceOwnership {},
    )
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
        instruction(
            forwarder_program,
            accounts::Upgrade {
                authority: *owner,
                config: derive_forwarder_config_pda(forwarder_program).0,
                program_data: derive_program_data_address(forwarder_program),
                upgraded_program: *forwarder_program,
                buffer: *buffer,
                spill: *spill,
                upgrade_authority: derive_upgrade_authority_pda(forwarder_program).0,
                rent: sysvar::rent::id(),
                clock: sysvar::clock::id(),
                bpf_loader_upgradeable: bpf_loader_upgradeable::id(),
                event_authority: derive_event_authority_pda(forwarder_program).0,
                program: *forwarder_program,
            },
            args::Upgrade {},
        ),
    ]
}

/// Build the forwarder's `version`, whose return data is the program's
/// version string.
pub fn version_ix(forwarder_program: &Pubkey) -> Instruction {
    instruction(forwarder_program, accounts::Version {}, args::Version {})
}

/// Build the forwarder's `close_escrow`: while the adapter whose state is
/// `pa_state` is paused, the owner drains `mint`'s escrow to the token
/// account `recipient_ata` and closes it, taking its rent.
pub fn close_escrow_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    pa_state: &Pubkey,
    mint: &Pubkey,
    recipient_ata: &Pubkey,
) -> Instruction {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    instruction(
        forwarder_program,
        accounts::CloseEscrow {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            escrow_ata: get_associated_token_address(&escrow_authority, mint),
            escrow_authority,
            recipient_ata: *recipient_ata,
            token_mint: *mint,
            token_program: spl_token_interface::id(),
            pa_state: *pa_state,
        },
        args::CloseEscrow {},
    )
}

/// Build the forwarder's `close_nonce_bitmaps_batch`: while the adapter
/// whose state is `pa_state` is paused, the owner closes the nonce bitmaps
/// `bitmaps`, taking their rent.
pub fn close_nonce_bitmaps_batch_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    pa_state: &Pubkey,
    bitmaps: &[Pubkey],
) -> Instruction {
    let mut ix = instruction(
        forwarder_program,
        accounts::CloseNonceBitmapsBatch {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            pa_state: *pa_state,
        },
        args::CloseNonceBitmapsBatch {},
    );
    ix.accounts.extend(
        bitmaps
            .iter()
            .map(|bitmap| AccountMeta::new(*bitmap, false)),
    );
    ix
}

/// Build the forwarder's `close_config`: while the adapter whose state is
/// `pa_state` is paused, the owner closes the config, taking its rent. The
/// forwarder serves nothing after it.
pub fn close_config_ix(
    forwarder_program: &Pubkey,
    owner: &Pubkey,
    pa_state: &Pubkey,
) -> Instruction {
    instruction(
        forwarder_program,
        accounts::CloseConfig {
            authority: *owner,
            config: derive_forwarder_config_pda(forwarder_program).0,
            pa_state: *pa_state,
        },
        args::CloseConfig {},
    )
}

/// The head of every forwarder CPI segment: the forwarder (segment marker),
/// its config, the instructions sysvar, then the forwarder's event authority
/// and the forwarder again, which its CPI events need.
pub(crate) fn forwarder_segment_head(
    forwarder_program: &Pubkey,
) -> [AccountMeta; FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS as usize] {
    [
        AccountMeta::new_readonly(*forwarder_program, false),
        AccountMeta::new_readonly(derive_forwarder_config_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(sysvar::instructions::id(), false),
        AccountMeta::new_readonly(derive_event_authority_pda(forwarder_program).0, false),
        AccountMeta::new_readonly(*forwarder_program, false),
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
    let (escrow_authority, _) = derive_forwarder_escrow_authority(forwarder_program);
    let mut accounts = forwarder_segment_head(forwarder_program).to_vec();
    accounts.extend([
        AccountMeta::new(
            get_associated_token_address(&escrow_authority, token_mint),
            false,
        ),
        AccountMeta::new(get_associated_token_address(recipient, token_mint), false),
        AccountMeta::new_readonly(escrow_authority, false),
        AccountMeta::new_readonly(spl_token_interface::id(), false),
    ]);
    accounts
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::constants::{FORWARDER_UNWRAP_NUM_ACCOUNTS, FORWARDER_WRAP_NUM_ACCOUNTS};
    use std::str::FromStr;

    fn canonical_spl_token_program_id() -> Pubkey {
        Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()
    }

    fn keys(accounts: &[AccountMeta]) -> Vec<Pubkey> {
        accounts.iter().map(|a| a.pubkey).collect()
    }

    /// Each instruction `the_builders_write_the_expected_instructions` builds,
    /// for its inputs (the program `key(9)`, other accounts `key(n)`): its
    /// program id, its data in hex, and its accounts (address, signer,
    /// writable), in order.
    const EXPECTED: &str = "\
== initialize
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data afaf6d1f0d989bed020202020202020202020202020202020202020202020202020202020202020203030303030303030303030303030303030303030303030303030303030303030404040404040404040404040404040404040404040404040404040404040404
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false true
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false false
acct Hibe1esmNpASGJBynhevaMybmY2gcGcPDrfz8zMZyw1J false true
acct 11111111111111111111111111111111 false false
acct BaNhKDjFJTKQhS1To1XK8eApMSYBoMUNky6wKrf4RX5N false false
acct CFLMyAiswuYuTmzEmBT9b9C1T1V6xqvWEfy7DT1T8MEP false false
acct BPFLoaderUpgradeab1e11111111111111111111111 false false
== init_nonce_bitmap
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data d60d7d7948dcf12a02020202020202020202020202020202020202020202020202020202020202020700000000000000
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct D5xBzsdS97MZ5wgZzqUadJ49QAJEScogLqPBfoMPAnH7 false true
acct 11111111111111111111111111111111 false false
== reinitialize
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 87ffcda1dfa25e890303030303030303030303030303030303030303030303030303030303030303
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true false
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false true
acct BaNhKDjFJTKQhS1To1XK8eApMSYBoMUNky6wKrf4RX5N false false
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false false
== transfer_ownership
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 41b1d749352d632f0202020202020202020202020202020202020202020202020202020202020202
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true false
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false true
acct BaNhKDjFJTKQhS1To1XK8eApMSYBoMUNky6wKrf4RX5N false false
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false false
== renounce_ownership
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 138f5b4f22a8ae7d
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true false
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false true
acct BaNhKDjFJTKQhS1To1XK8eApMSYBoMUNky6wKrf4RX5N false false
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false false
== upgrade
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data efee394acd57a51a
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true false
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false false
acct Hibe1esmNpASGJBynhevaMybmY2gcGcPDrfz8zMZyw1J false true
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false true
acct 8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR false true
acct CktRuQ2mttgRGkXJtyksdKHjUdc2C4TgDzyB98oEzy8 false true
acct CFLMyAiswuYuTmzEmBT9b9C1T1V6xqvWEfy7DT1T8MEP false false
acct SysvarRent111111111111111111111111111111111 false false
acct SysvarC1ock11111111111111111111111111111111 false false
acct BPFLoaderUpgradeab1e11111111111111111111111 false false
acct BaNhKDjFJTKQhS1To1XK8eApMSYBoMUNky6wKrf4RX5N false false
acct cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN false false
== version
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 7641c3c681d8fcc0
== close_escrow
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 8bab5e92bf5b9032
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false false
acct CiDJk7HUU3KHSErtNP7ktQwpjScr5te9eHhpXwEUcdJC false true
acct 7VW1B28SQgEXDytdUCBxi6NWS7kCigKCKZe7qeQ4Jm1y false false
acct GgBaCs3NCBuZN12kCJgAW63ydqohFkHEdfdEXBPzLHq false true
acct CktRuQ2mttgRGkXJtyksdKHjUdc2C4TgDzyB98oEzy8 false false
acct TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA false false
acct 8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR false false
== close_nonce_bitmaps_batch
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data d4430fbee28d8e65
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false false
acct 8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR false false
acct LbUiWL3xVV8hTFYBVdbTNrpDo41NKS6o3LHHuDzjfcY false true
acct QWmroo4YnnMqYW3cnxWkFdaTxGD3P7vMSzwMHGbUzwF false true
== close_config
program cGfHiC6Kgg3FpFZvgwGcswsCRtp4aBP2fzuXRQPizuN
data 9109489d5f7d3d55
acct 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi true true
acct 8uZ1Xjv8Jsyfi98uTWryPLi1GrLijtpsgA45EqGhJMEb false true
acct 8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR false false
";

    pub(crate) fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    /// `ix` in `EXPECTED`'s form.
    fn written(name: &str, ix: &Instruction) -> String {
        let data: String = ix.data.iter().map(|b| format!("{b:02x}")).collect();
        let mut out = format!("== {name}\nprogram {}\ndata {data}\n", ix.program_id);
        for meta in &ix.accounts {
            out += &format!(
                "acct {} {} {}\n",
                meta.pubkey, meta.is_signer, meta.is_writable
            );
        }
        out
    }

    /// `ix` is the entry `name` of `expected`, a list in `EXPECTED`'s form.
    pub(crate) fn assert_is_expected(expected: &str, name: &str, ix: &Instruction) {
        let start = expected
            .find(&format!("== {name}\n"))
            .unwrap_or_else(|| panic!("no expected instruction {name}"));
        let end = expected[start + 3..]
            .find("== ")
            .map_or(expected.len(), |i| start + 3 + i);
        let built = written(name, ix);
        println!("{built}");
        assert_eq!(built, expected[start..end], "{name}");
    }

    #[test]
    fn the_builders_write_the_expected_instructions() {
        let p = key(9);
        for (name, ix) in [
            (
                "initialize",
                initialize_ix(&p, &key(1), &key(2), [3; 32], &key(4)),
            ),
            (
                "init_nonce_bitmap",
                init_nonce_bitmap_ix(&p, &key(1), &key(2), 7),
            ),
            ("reinitialize", reinitialize_ix(&p, &key(1), [3; 32])),
            (
                "transfer_ownership",
                transfer_ownership_ix(&p, &key(1), &key(2)),
            ),
            ("renounce_ownership", renounce_ownership_ix(&p, &key(1))),
            (
                "upgrade",
                upgrade_ixs(&p, &key(1), &key(2), &key(3))[1].clone(),
            ),
            ("version", version_ix(&p)),
            (
                "close_escrow",
                close_escrow_ix(&p, &key(1), &key(2), &key(3), &key(4)),
            ),
            (
                "close_nonce_bitmaps_batch",
                close_nonce_bitmaps_batch_ix(&p, &key(1), &key(2), &[key(5), key(6)]),
            ),
            ("close_config", close_config_ix(&p, &key(1), &key(2))),
        ] {
            assert_is_expected(EXPECTED, name, &ix);
        }
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
    fn nonce_word_index_covers_256_nonces_per_word() {
        assert_eq!(nonce_word_index(0), 0);
        assert_eq!(nonce_word_index(255), 0);
        assert_eq!(nonce_word_index(256), 1);
        assert_eq!(nonce_word_index(u64::MAX), u64::MAX / 256);
    }

    #[test]
    fn init_nonce_bitmap_ix_matches_the_forwarder_idl() {
        let forwarder = crate::generated::spl_token_forwarder::ID;
        let (payer, user) = (Pubkey::new_unique(), Pubkey::new_unique());
        let ix = init_nonce_bitmap_ix(&forwarder, &payer, &user, 7);
        let args = [user.as_ref(), &7u64.to_le_bytes()].concat();
        let by_name = assert_matches_the_idl(&crate::idl::idl(), &ix, "init_nonce_bitmap", &args);
        assert_eq!(by_name("payer"), payer);
        // The IDL's seeds of the bitmap name args it cannot resolve.
        assert_eq!(
            by_name("nonce_bitmap"),
            derive_nonce_bitmap_pda(&forwarder, &user, 7).0
        );
        assert_eq!(ix.accounts.len(), 3);
    }

    /// Checks `ix` against the instruction `name` of `idl`, either build's:
    /// the data is its discriminator then `args`, and each account has the
    /// IDL's flags, its fixed address, or its PDA from the IDL's seeds when
    /// they are all constants. Returns the address `ix` passes for each IDL
    /// account name.
    /// The address the IDL gives the instruction account `account` of
    /// `program`: its fixed address, or its PDA when every seed is a
    /// constant. None for a PDA with an `arg` seed: the IDLs name those
    /// seeds' args with the handlers' underscored names, so no arg resolves
    /// them.
    pub(crate) fn idl_address(account: &serde_json::Value, program: &Pubkey) -> Option<Pubkey> {
        if let Some(address) = account["address"].as_str() {
            return Some(address.parse().unwrap());
        }
        let seeds = account["pda"]["seeds"].as_array()?;
        if !seeds.iter().all(|seed| seed["kind"] == "const") {
            return None;
        }
        let seeds: Vec<Vec<u8>> = seeds
            .iter()
            .map(|seed| serde_json::from_value(seed["value"].clone()).unwrap())
            .collect();
        let seeds: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
        Some(Pubkey::find_program_address(&seeds, program).0)
    }

    pub(crate) fn assert_matches_the_idl(
        idl: &serde_json::Value,
        ix: &Instruction,
        name: &str,
        args: &[u8],
    ) -> impl Fn(&str) -> Pubkey {
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
            if let Some(declared) = idl_address(account, &ix.program_id) {
                assert_eq!(meta.pubkey, declared, "{account_name} address");
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
    pub(crate) fn assert_event_accounts(by_name: &impl Fn(&str) -> Pubkey, forwarder: &Pubkey) {
        assert_eq!(
            by_name("event_authority"),
            derive_event_authority_pda(forwarder).0
        );
        assert_eq!(by_name("program"), *forwarder);
    }

    #[test]
    fn initialize_matches_the_forwarders_idl() {
        let forwarder = crate::generated::spl_token_forwarder::ID;
        let (authority, adapter, owner) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let ix = initialize_ix(&forwarder, &authority, &adapter, [7; 32], &owner);
        let args = [adapter.to_bytes().as_slice(), &[7; 32], &owner.to_bytes()].concat();
        let by_name = assert_matches_the_idl(&crate::idl::idl(), &ix, "initialize", &args);
        assert_eq!(by_name("authority"), authority);
        assert_eq!(ix.accounts.len(), 8);
    }

    #[test]
    fn the_owner_only_instructions_match_the_forwarders_idl() {
        let forwarder = crate::generated::spl_token_forwarder::ID;
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
            let by_name = assert_matches_the_idl(&crate::idl::idl(), &ix, name, &args);
            assert_eq!(by_name("authority"), owner, "{name}");
            assert_event_accounts(&by_name, &forwarder);
            assert_eq!(ix.accounts.len(), 4, "{name}");
        }
    }

    #[test]
    fn upgrade_sets_its_compute_budget_and_matches_the_forwarders_idl() {
        let forwarder = crate::generated::spl_token_forwarder::ID;
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
        let by_name = assert_matches_the_idl(&crate::idl::idl(), &ix, "upgrade", &[]);
        assert_eq!(by_name("authority"), owner);
        assert_eq!(by_name("buffer"), buffer);
        assert_eq!(by_name("spill"), spill);
        assert_event_accounts(&by_name, &forwarder);
        assert_eq!(ix.accounts.len(), 12);
    }

    #[test]
    fn version_matches_the_forwarders_idl() {
        let forwarder = crate::generated::spl_token_forwarder::ID;
        let ix = version_ix(&forwarder);
        let _ = assert_matches_the_idl(&crate::idl::idl(), &ix, "version", &[]);
        assert!(ix.accounts.is_empty());
    }

    #[test]
    fn the_closing_instructions_match_the_forwarders_idl() {
        let forwarder = crate::generated::spl_token_forwarder::ID;
        let (owner, pa_state, mint, recipient_ata) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );

        let ix = close_escrow_ix(&forwarder, &owner, &pa_state, &mint, &recipient_ata);
        let by_name = assert_matches_the_idl(&crate::idl::idl(), &ix, "close_escrow", &[]);
        let escrow_authority = derive_forwarder_escrow_authority(&forwarder).0;
        assert_eq!(by_name("authority"), owner);
        assert_eq!(
            by_name("escrow_ata"),
            get_associated_token_address(&escrow_authority, &mint)
        );
        assert_eq!(by_name("recipient_ata"), recipient_ata);
        assert_eq!(by_name("token_mint"), mint);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_eq!(ix.accounts.len(), 8);

        let ix = close_config_ix(&forwarder, &owner, &pa_state);
        let by_name = assert_matches_the_idl(&crate::idl::idl(), &ix, "close_config", &[]);
        assert_eq!(by_name("authority"), owner);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_eq!(ix.accounts.len(), 3);

        let bitmaps = [Pubkey::new_unique(), Pubkey::new_unique()];
        let ix = close_nonce_bitmaps_batch_ix(&forwarder, &owner, &pa_state, &bitmaps);
        let by_name =
            assert_matches_the_idl(&crate::idl::idl(), &ix, "close_nonce_bitmaps_batch", &[]);
        assert_eq!(by_name("authority"), owner);
        assert_eq!(by_name("pa_state"), pa_state);
        assert_eq!(
            ix.accounts[3..].to_vec(),
            bitmaps
                .map(|bitmap| AccountMeta::new(bitmap, false))
                .to_vec(),
            "the bitmaps, writable, as remaining accounts"
        );
    }
}
