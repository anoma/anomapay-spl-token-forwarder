//! The forwarder's part of a settlement. The proof commits each external
//! call's program, instruction data and account count, but not the accounts,
//! so a submitter supplies them: each call's CPI segment, and the
//! instructions the settlement transaction carries before the settlement.
//! Everything here follows from the transaction's external calls and where
//! the settlement carries the forwarder's preceding instructions.

use std::collections::BTreeSet;

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use anoma_pa_solana_client::settlement::CallAccounts;
use solana_ed25519_program::new_ed25519_instruction_with_signature;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent;

use crate::emergency_migrating::{
    build_migrate_forwarder_accounts, derive_migration_record_pda, init_migration_record_ix,
};
use crate::forwarder::{
    build_unwrap_forwarder_accounts, build_wrap_forwarder_accounts, init_nonce_bitmap_ix,
    nonce_word_index,
};
use crate::input::{decode_forwarder_input, ForwarderInput, InputError, WrapInput};
use crate::pda::derive_nonce_bitmap_pda;

/// The calls cannot be settled through the forwarder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallsError {
    /// Call `call` is to `program`, not to the forwarder.
    NotTheForwarder { call: usize, program: Pubkey },
    /// Call `call`'s instruction data is not a forwarder input.
    Input { call: usize, error: InputError },
    /// Call `call` commits `committed` accounts; the forwarder's segment for
    /// its input has `segment`.
    AccountCount {
        call: usize,
        committed: u8,
        segment: usize,
    },
    /// The wraps name the signature-check indices `named`, sorted, which are
    /// not `start..start + k` for k wraps whose checks start at `start`.
    SignatureCheckIndices { named: Vec<u8>, start: usize },
}

impl core::fmt::Display for CallsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CallsError::NotTheForwarder { call, program } => {
                write!(
                    f,
                    "external call {call} is to {program}, not to the forwarder"
                )
            }
            CallsError::Input { call, error } => {
                write!(f, "external call {call}'s forwarder input: {error}")
            }
            CallsError::AccountCount {
                call,
                committed,
                segment,
            } => write!(
                f,
                "external call {call} commits {committed} accounts; the forwarder's segment has \
                 {segment}"
            ),
            CallsError::SignatureCheckIndices { named, start } => write!(
                f,
                "the wraps name signature-check indices {named:?}; the settlement carries their \
                 {} checks at indices {start}..{}",
                named.len(),
                start + named.len()
            ),
        }
    }
}

impl std::error::Error for CallsError {}

/// The inputs of `calls`, every one to the forwarder `forwarder`.
fn inputs(
    forwarder: &Pubkey,
    calls: &[SolanaExternalCall],
) -> Result<Vec<ForwarderInput>, CallsError> {
    calls
        .iter()
        .enumerate()
        .map(|(call, external_call)| {
            let program = Pubkey::new_from_array(external_call.program_id);
            if program != *forwarder {
                return Err(CallsError::NotTheForwarder { call, program });
            }
            decode_forwarder_input(&external_call.instruction_data)
                .map_err(|error| CallsError::Input { call, error })
        })
        .collect()
}

/// The nonce bitmap `wrap` uses through the forwarder `forwarder`, and its
/// word.
fn nonce_bitmap(forwarder: &Pubkey, wrap: &WrapInput) -> (Pubkey, u64) {
    let word = nonce_word_index(wrap.nonce);
    let user = Pubkey::new_from_array(wrap.user);
    (derive_nonce_bitmap_pda(forwarder, &user, word).0, word)
}

/// What a settlement through the adapter `protocol_adapter` passes the
/// forwarder `forwarder` for `calls`, every one to it, `payer` paying for
/// the accounts the calls need created. Every account creation does nothing
/// when the account exists, so the settlement carries them whatever the
/// chain holds.
///
/// A wrap's input names the absolute index of its signature check in the
/// settlement transaction. The checks come first among the forwarder's
/// preceding instructions, which the settlement carries from index
/// `preceding_start` on, so the indices k wraps name must be exactly
/// `preceding_start..preceding_start + k`. Each check is built from the wrap
/// input's signature ([`WrapInput::signature`](crate::WrapInput::signature)).
pub fn settlement_calls(
    forwarder: &Pubkey,
    protocol_adapter: &Pubkey,
    payer: &Pubkey,
    calls: &[SolanaExternalCall],
    preceding_start: usize,
) -> Result<CallAccounts, CallsError> {
    let mut checks: Vec<(u8, Instruction)> = Vec::new();
    let mut accounts_needed: Vec<Instruction> = Vec::new();
    // The bitmaps and migration records the calls need, each created once.
    let mut created: BTreeSet<Pubkey> = BTreeSet::new();
    let mut segments = Vec::with_capacity(calls.len());

    for (call, (external_call, input)) in calls.iter().zip(inputs(forwarder, calls)?).enumerate() {
        let segment = match input {
            ForwarderInput::Wrap(wrap) => {
                let user = Pubkey::new_from_array(wrap.user);
                let message = wrap.to_message(&forwarder.to_bytes()).base64_digest();
                checks.push((
                    wrap.ed25519_ix_index,
                    new_ed25519_instruction_with_signature(
                        message.as_bytes(),
                        &wrap.signature,
                        &wrap.user,
                    ),
                ));
                // A wrap needs the bitmap of its nonce's word, and the
                // adapter's CPI carries no signer that could create it. Two
                // wraps of one word create it once.
                let (bitmap, word) = nonce_bitmap(forwarder, &wrap);
                if created.insert(bitmap) {
                    accounts_needed.push(init_nonce_bitmap_ix(forwarder, payer, &user, word));
                }
                build_wrap_forwarder_accounts(
                    forwarder,
                    &user,
                    &Pubkey::new_from_array(wrap.token_mint),
                    wrap.nonce,
                )
            }
            ForwarderInput::Unwrap(unwrap) => {
                let recipient = Pubkey::new_from_array(unwrap.recipient);
                let mint = Pubkey::new_from_array(unwrap.token_mint);
                // The forwarder's transfer needs the recipient's token account.
                accounts_needed.push(create_associated_token_account_idempotent(
                    payer,
                    &recipient,
                    &mint,
                    &spl_token_interface::id(),
                ));
                build_unwrap_forwarder_accounts(forwarder, &recipient, &mint)
            }
            ForwarderInput::Migrate(migration) => {
                // Each entry needs its migration record, which, as a
                // bitmap, the adapter's CPI cannot create.
                for entry in &migration.entries {
                    let (record, _) = derive_migration_record_pda(forwarder, &entry.nullifier);
                    if created.insert(record) {
                        accounts_needed.push(init_migration_record_ix(
                            forwarder,
                            payer,
                            &entry.nullifier,
                        ));
                    }
                }
                build_migrate_forwarder_accounts(forwarder, protocol_adapter, &migration.entries)
            }
        };
        if segment.len() != usize::from(external_call.num_accounts) {
            return Err(CallsError::AccountCount {
                call,
                committed: external_call.num_accounts,
                segment: segment.len(),
            });
        }
        segments.push(segment);
    }

    checks.sort_by_key(|(index, _)| *index);
    if checks
        .iter()
        .map(|(index, _)| usize::from(*index))
        .ne(preceding_start..preceding_start + checks.len())
    {
        return Err(CallsError::SignatureCheckIndices {
            named: checks.iter().map(|(index, _)| *index).collect(),
            start: preceding_start,
        });
    }

    Ok(CallAccounts {
        segments,
        preceding: checks
            .into_iter()
            .map(|(_, check)| check)
            .chain(accounts_needed)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{
        forwarder_migrate_num_accounts, FORWARDER_UNWRAP_NUM_ACCOUNTS, FORWARDER_WRAP_NUM_ACCOUNTS,
    };
    use crate::input::{
        encode_migrate_forwarder_input, encode_unwrap_forwarder_input, encode_wrap_forwarder_input,
        MigrateEntry, MigrateInput, WrapInput,
    };
    use anoma_pa_solana_client::external_call::OutputMode;

    const FORWARDER: Pubkey = Pubkey::new_from_array([7; 32]);
    const PAYER: Pubkey = Pubkey::new_from_array([8; 32]);
    const ADAPTER: Pubkey = Pubkey::new_from_array([9; 32]);

    fn wrap(user: u8, nonce: u64, ed25519_ix_index: u8) -> WrapInput {
        WrapInput {
            token_mint: [1; 32],
            amount: 1_000,
            user: [user; 32],
            nonce,
            deadline: 1_700_000_000,
            action_tree_root: [3; 32],
            signature: [user ^ 0x55; 64],
            ed25519_ix_index,
        }
    }

    fn call(program: Pubkey, instruction_data: Vec<u8>, num_accounts: u8) -> SolanaExternalCall {
        SolanaExternalCall {
            program_id: program.to_bytes(),
            instruction_data,
            expected_output: vec![],
            output_mode: OutputMode::ReturnData,
            num_accounts,
        }
    }

    fn wrap_call(input: &WrapInput) -> SolanaExternalCall {
        call(
            FORWARDER,
            encode_wrap_forwarder_input(input),
            FORWARDER_WRAP_NUM_ACCOUNTS,
        )
    }

    /// The signature check a wrap's settlement must carry: over the wrap
    /// message, by the user, with the input's signature.
    fn check_of(input: &WrapInput) -> Instruction {
        let message = input.to_message(&FORWARDER.to_bytes()).base64_digest();
        new_ed25519_instruction_with_signature(message.as_bytes(), &input.signature, &input.user)
    }

    #[test]
    fn a_wrap_carries_its_signature_check_then_its_bitmaps_init() {
        let input = wrap(2, 9, 0);
        let settlement =
            settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &[wrap_call(&input)], 0).unwrap();
        assert_eq!(
            settlement.preceding,
            vec![
                check_of(&input),
                init_nonce_bitmap_ix(&FORWARDER, &PAYER, &Pubkey::new_from_array(input.user), 0),
            ]
        );
        assert_eq!(
            settlement.segments,
            vec![build_wrap_forwarder_accounts(
                &FORWARDER,
                &Pubkey::new_from_array(input.user),
                &Pubkey::new_from_array(input.token_mint),
                input.nonce
            )]
        );
    }

    #[test]
    fn two_wraps_of_one_bitmap_create_it_once() {
        let (first, second) = (wrap(2, 9, 0), wrap(2, 10, 1));
        let calls = [wrap_call(&first), wrap_call(&second)];
        let settlement = settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0).unwrap();
        assert_eq!(settlement.preceding.len(), 3, "two checks, one bitmap init");
    }

    #[test]
    fn the_signature_checks_sit_at_the_indices_the_wraps_name() {
        // The second call's wrap names index 0, the first's index 1.
        let (first, second) = (wrap(2, 9, 1), wrap(4, 9, 0));
        let calls = [wrap_call(&first), wrap_call(&second)];
        let settlement = settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0).unwrap();
        assert_eq!(
            settlement.preceding[..2],
            [check_of(&second), check_of(&first)]
        );
    }

    #[test]
    fn wraps_naming_indices_other_than_the_first_ones_are_refused() {
        let calls = [wrap_call(&wrap(2, 9, 1))];
        assert_eq!(
            settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0),
            Err(CallsError::SignatureCheckIndices {
                named: vec![1],
                start: 0
            })
        );
    }

    /// Another forwarder's two preceding instructions come first, so this
    /// forwarder's checks start at index 2, where its wraps name them.
    #[test]
    fn the_signature_checks_start_where_the_forwarders_preceding_instructions_do() {
        let (first, second) = (wrap(2, 9, 3), wrap(4, 9, 2));
        let calls = [wrap_call(&first), wrap_call(&second)];
        let settlement = settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 2).unwrap();
        assert_eq!(
            settlement.preceding[..2],
            [check_of(&second), check_of(&first)]
        );
        // Indices from 0 name the other forwarder's instructions.
        assert_eq!(
            settlement_calls(
                &FORWARDER,
                &ADAPTER,
                &PAYER,
                &[wrap_call(&wrap(2, 9, 0)), wrap_call(&wrap(4, 9, 1))],
                2
            ),
            Err(CallsError::SignatureCheckIndices {
                named: vec![0, 1],
                start: 2
            })
        );
    }

    #[test]
    fn an_unwrap_creates_the_recipients_token_account() {
        let (mint, recipient) = ([1; 32], [5; 32]);
        let calls = [call(
            FORWARDER,
            encode_unwrap_forwarder_input(&mint, 100, &recipient),
            FORWARDER_UNWRAP_NUM_ACCOUNTS,
        )];
        let (mint, recipient) = (
            Pubkey::new_from_array(mint),
            Pubkey::new_from_array(recipient),
        );
        assert_eq!(
            settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0).unwrap(),
            CallAccounts {
                segments: vec![build_unwrap_forwarder_accounts(
                    &FORWARDER, &recipient, &mint
                )],
                preceding: vec![create_associated_token_account_idempotent(
                    &PAYER,
                    &recipient,
                    &mint,
                    &spl_token_interface::id()
                )],
            }
        );
    }

    #[test]
    fn a_call_to_another_program_is_refused() {
        let other = Pubkey::new_from_array([9; 32]);
        let calls = [call(
            other,
            encode_wrap_forwarder_input(&wrap(2, 9, 0)),
            FORWARDER_WRAP_NUM_ACCOUNTS,
        )];
        let refusal = CallsError::NotTheForwarder {
            call: 0,
            program: other,
        };
        assert_eq!(
            settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0),
            Err(refusal)
        );
    }

    /// A migration's entries, of which the first and the last name one
    /// nullifier: the settlement creates each record once, and the segment
    /// lists every entry's accounts.
    #[test]
    fn a_migration_carries_each_records_init_once_and_its_segment() {
        let entry = |nullifier: u8| MigrateEntry {
            nullifier: [nullifier; 32],
            commitment_tree_root: [2; 32],
            vulnerable_logic_ref: [3; 32],
            forwarder: FORWARDER.to_bytes(),
        };
        let migration = MigrateInput {
            token_mint: [1; 32],
            quantity: 30,
            entries: vec![entry(4), entry(5), entry(4)],
        };
        let calls = [call(
            FORWARDER,
            encode_migrate_forwarder_input(&migration),
            forwarder_migrate_num_accounts(3).unwrap(),
        )];
        let settlement = settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0).unwrap();
        assert_eq!(
            settlement.preceding,
            vec![
                init_migration_record_ix(&FORWARDER, &PAYER, &[4; 32]),
                init_migration_record_ix(&FORWARDER, &PAYER, &[5; 32]),
            ]
        );
        assert_eq!(
            settlement.segments,
            vec![build_migrate_forwarder_accounts(
                &FORWARDER,
                &ADAPTER,
                &migration.entries
            )]
        );
    }

    #[test]
    fn a_call_committing_another_account_count_is_refused() {
        let calls = [call(
            FORWARDER,
            encode_wrap_forwarder_input(&wrap(2, 9, 0)),
            FORWARDER_WRAP_NUM_ACCOUNTS + 1,
        )];
        assert_eq!(
            settlement_calls(&FORWARDER, &ADAPTER, &PAYER, &calls, 0),
            Err(CallsError::AccountCount {
                call: 0,
                committed: FORWARDER_WRAP_NUM_ACCOUNTS + 1,
                segment: usize::from(FORWARDER_WRAP_NUM_ACCOUNTS),
            })
        );
    }
}
