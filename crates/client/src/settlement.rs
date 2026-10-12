//! The forwarder's part of a settlement. The proof commits each external
//! call's program, instruction data and account count, but not the accounts,
//! so a submitter supplies them: each call's CPI segment, and the
//! instructions the settlement transaction carries before the settlement.
//! Everything here follows from the transaction's external calls, except
//! which nonce bitmaps exist, which the submitter reads from the chain.

use std::collections::BTreeSet;

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use solana_ed25519_program::new_ed25519_instruction_with_signature;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent;

use crate::forwarder::{
    build_unwrap_forwarder_accounts, build_wrap_forwarder_accounts, init_nonce_bitmap_ix,
    nonce_word_index,
};
use crate::input::{decode_forwarder_input, ForwarderInput, InputError, WrapInput};
use crate::pda::derive_nonce_bitmap_pda;

/// What a settlement passes the forwarder for its calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettlementCalls {
    /// Each call's CPI segment, in call order.
    pub segments: Vec<Vec<AccountMeta>>,
    /// The instructions before the settlement: the wraps' ed25519 signature
    /// checks, each at the index its wrap names, then the accounts the calls
    /// need to exist.
    pub preceding: Vec<Instruction>,
}

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
    /// The wraps name these signature-check indices, sorted, which are not
    /// `0..k` for k wraps.
    SignatureCheckIndices(Vec<u8>),
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
            CallsError::SignatureCheckIndices(named) => write!(
                f,
                "the wraps name signature-check indices {named:?}; a settlement carries its {} \
                 checks first, at indices 0..{}",
                named.len(),
                named.len()
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

/// The nonce bitmaps the wraps among `calls` use, every call to the forwarder
/// `forwarder`, sorted and without repeats: the accounts a submitter reads to
/// learn which are absent.
pub fn wrap_nonce_bitmaps(
    forwarder: &Pubkey,
    calls: &[SolanaExternalCall],
) -> Result<Vec<Pubkey>, CallsError> {
    let bitmaps: BTreeSet<Pubkey> = inputs(forwarder, calls)?
        .iter()
        .filter_map(|input| match input {
            ForwarderInput::Wrap(wrap) => Some(nonce_bitmap(forwarder, wrap).0),
            ForwarderInput::Unwrap(_) => None,
        })
        .collect();
    Ok(bitmaps.into_iter().collect())
}

/// What a settlement passes the forwarder `forwarder` for `calls`, every one
/// to it, `payer` paying for the accounts the calls need created;
/// `absent_bitmaps` are those of [`wrap_nonce_bitmaps`] the chain lacks.
///
/// A wrap's input names the absolute index of its signature check in the
/// settlement transaction. The checks come first, so the indices k wraps
/// name must be exactly `0..k`. Each check is built from the wrap input's
/// signature ([`WrapInput::signature`](crate::WrapInput::signature)).
pub fn settlement_calls(
    forwarder: &Pubkey,
    payer: &Pubkey,
    calls: &[SolanaExternalCall],
    mut absent_bitmaps: BTreeSet<Pubkey>,
) -> Result<SettlementCalls, CallsError> {
    let mut checks: Vec<(u8, Instruction)> = Vec::new();
    let mut accounts_needed: Vec<Instruction> = Vec::new();
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
                // adapter's CPI carries no signer that could create it.
                // Removed once created, so two wraps of a word create it once.
                let (bitmap, word) = nonce_bitmap(forwarder, &wrap);
                if absent_bitmaps.remove(&bitmap) {
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
        .ne(0..checks.len())
    {
        return Err(CallsError::SignatureCheckIndices(
            checks.iter().map(|(index, _)| *index).collect(),
        ));
    }

    Ok(SettlementCalls {
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
    use crate::constants::{FORWARDER_UNWRAP_NUM_ACCOUNTS, FORWARDER_WRAP_NUM_ACCOUNTS};
    use crate::input::{encode_unwrap_forwarder_input, encode_wrap_forwarder_input, WrapInput};
    use anoma_pa_solana_client::external_call::OutputMode;

    const FORWARDER: Pubkey = Pubkey::new_from_array([7; 32]);
    const PAYER: Pubkey = Pubkey::new_from_array([8; 32]);

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

    fn bitmap_of(input: &WrapInput) -> Pubkey {
        derive_nonce_bitmap_pda(
            &FORWARDER,
            &Pubkey::new_from_array(input.user),
            nonce_word_index(input.nonce),
        )
        .0
    }

    /// The signature check a wrap's settlement must carry: over the wrap
    /// message, by the user, with the input's signature.
    fn check_of(input: &WrapInput) -> Instruction {
        let message = input.to_message(&FORWARDER.to_bytes()).base64_digest();
        new_ed25519_instruction_with_signature(message.as_bytes(), &input.signature, &input.user)
    }

    #[test]
    fn a_first_wrap_carries_its_signature_check_then_creates_its_bitmap() {
        let input = wrap(2, 9, 0);
        let calls = [wrap_call(&input)];
        let bitmaps = wrap_nonce_bitmaps(&FORWARDER, &calls).unwrap();
        assert_eq!(bitmaps, vec![bitmap_of(&input)]);

        let settlement =
            settlement_calls(&FORWARDER, &PAYER, &calls, bitmaps.into_iter().collect()).unwrap();
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
    fn a_wrap_whose_bitmap_exists_carries_only_its_signature_check() {
        let input = wrap(2, 9, 0);
        let settlement =
            settlement_calls(&FORWARDER, &PAYER, &[wrap_call(&input)], BTreeSet::new()).unwrap();
        assert_eq!(settlement.preceding, vec![check_of(&input)]);
    }

    #[test]
    fn two_wraps_of_one_bitmap_create_it_once() {
        let (first, second) = (wrap(2, 9, 0), wrap(2, 10, 1));
        let calls = [wrap_call(&first), wrap_call(&second)];
        let bitmaps = wrap_nonce_bitmaps(&FORWARDER, &calls).unwrap();
        assert_eq!(bitmaps.len(), 1, "one word, one bitmap");
        let settlement =
            settlement_calls(&FORWARDER, &PAYER, &calls, bitmaps.into_iter().collect()).unwrap();
        assert_eq!(settlement.preceding.len(), 3, "two checks, one bitmap init");
    }

    #[test]
    fn the_signature_checks_sit_at_the_indices_the_wraps_name() {
        // The second call's wrap names index 0, the first's index 1.
        let (first, second) = (wrap(2, 9, 1), wrap(4, 9, 0));
        let calls = [wrap_call(&first), wrap_call(&second)];
        let settlement = settlement_calls(&FORWARDER, &PAYER, &calls, BTreeSet::new()).unwrap();
        assert_eq!(
            settlement.preceding,
            vec![check_of(&second), check_of(&first)]
        );
    }

    #[test]
    fn wraps_naming_indices_other_than_the_first_ones_are_refused() {
        let calls = [wrap_call(&wrap(2, 9, 1))];
        assert_eq!(
            settlement_calls(&FORWARDER, &PAYER, &calls, BTreeSet::new()),
            Err(CallsError::SignatureCheckIndices(vec![1]))
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
        assert_eq!(
            wrap_nonce_bitmaps(&FORWARDER, &calls).unwrap(),
            Vec::<Pubkey>::new()
        );
        let (mint, recipient) = (
            Pubkey::new_from_array(mint),
            Pubkey::new_from_array(recipient),
        );
        assert_eq!(
            settlement_calls(&FORWARDER, &PAYER, &calls, BTreeSet::new()).unwrap(),
            SettlementCalls {
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
        assert_eq!(wrap_nonce_bitmaps(&FORWARDER, &calls), Err(refusal.clone()));
        assert_eq!(
            settlement_calls(&FORWARDER, &PAYER, &calls, BTreeSet::new()),
            Err(refusal)
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
            settlement_calls(&FORWARDER, &PAYER, &calls, BTreeSet::new()),
            Err(CallsError::AccountCount {
                call: 0,
                committed: FORWARDER_WRAP_NUM_ACCOUNTS + 1,
                segment: usize::from(FORWARDER_WRAP_NUM_ACCOUNTS),
            })
        );
    }
}
