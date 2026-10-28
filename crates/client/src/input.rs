//! The forwarder's instruction data: the op byte and the wrap or unwrap
//! input, which a resource's external call carries as its instruction data.
//! Each input after the op byte is the Borsh encoding of the program's own
//! input type, which the program parses with `try_from_slice`.

use anchor_lang::{AnchorDeserialize, AnchorSerialize};

use crate::generated::emergency_migrating_spl_token_forwarder::constants::OP_MIGRATE;
use crate::generated::spl_token_forwarder::constants::{OP_UNWRAP, OP_WRAP};
use crate::wrap_message::WrapMessage;

/// `op`, then the Borsh encoding of `input`.
fn with_op(op: u8, input: &impl AnchorSerialize) -> Vec<u8> {
    let mut buf = vec![op];
    input
        .serialize(&mut buf)
        .expect("a Vec takes any input of fewer than 2^32 migration entries");
    buf
}

/// Build the 186-byte forwarder instruction data for the wrap `input`.
///
/// Layout: `op(1) + token_mint(32) + amount_le(8) + user(32) + nonce_le(8) +
/// deadline_le_i64(8) + action_tree_root(32) + signature(64) +
/// ed25519_ix_index(1)`.
pub fn encode_wrap_forwarder_input(input: &WrapInput) -> Vec<u8> {
    with_op(OP_WRAP, input)
}

/// Build the 73-byte forwarder instruction data for an unwrap.
///
/// Layout: `op(1) + token_mint(32) + amount_le(8) + recipient(32)`.
pub fn encode_unwrap_forwarder_input(
    token_mint: &[u8; 32],
    amount: u64,
    recipient: &[u8; 32],
) -> Vec<u8> {
    with_op(
        OP_UNWRAP,
        &UnwrapInput {
            token_mint: *token_mint,
            amount,
            recipient: *recipient,
        },
    )
}

/// Build the forwarder instruction data for the migration `input`, which
/// the emergency-migrating build accepts.
///
/// Layout: `op(1) + token_mint(32) + quantity_le(8) +
/// entry_count_le_u32(4)`, then per entry `nullifier(32) +
/// commitment_tree_root(32) + vulnerable_logic_ref(32) + forwarder(32)`.
pub fn encode_migrate_forwarder_input(input: &MigrateInput) -> Vec<u8> {
    with_op(OP_MIGRATE, input)
}

/// A forwarder call's instruction data, decoded: what a submitter reads from
/// the call a proof commits to supply its accounts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwarderInput {
    Wrap(WrapInput),
    Unwrap(UnwrapInput),
    Migrate(MigrateInput),
}

/// A migration's input, the EVM draft's `(IERC20 token, uint128 quantity,
/// MigrateEntry[] entries)`: the forwarder records the entries' nullifiers,
/// which moves no tokens. `quantity` is the batch's total in SPL Token's
/// width, which the trigger's logic checks and the forwarder does not read.
#[derive(Clone, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub struct MigrateInput {
    pub token_mint: [u8; 32],
    pub quantity: u64,
    pub entries: Vec<MigrateEntry>,
}

/// One resource of a migration batch, as the EVM draft's `MigrateEntry`.
#[derive(Clone, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub struct MigrateEntry {
    pub nullifier: [u8; 32],
    /// The commitment tree root the resource is proven against.
    pub commitment_tree_root: [u8; 32],
    /// The resource's logic ref, which the forwarder lists as vulnerable.
    pub vulnerable_logic_ref: [u8; 32],
    /// The forwarder in the resource's label.
    pub forwarder: [u8; 32],
}

/// A wrap's input: the forwarder escrows `amount` of `token_mint` from
/// `user`'s token account, authorized by the user's ed25519 signature, which
/// the settlement transaction verifies in the ed25519 instruction at
/// `ed25519_ix_index`.
#[derive(Clone, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub struct WrapInput {
    pub token_mint: [u8; 32],
    pub amount: u64,
    pub user: [u8; 32],
    pub nonce: u64,
    /// Signed to match the forwarder's `WrapInput::deadline: i64`.
    pub deadline: i64,
    pub action_tree_root: [u8; 32],
    /// The user's ed25519 signature over the wrap message.
    ///
    /// The forwarder program does not read it, and that is intended: a
    /// program cannot call the ed25519 verifier (precompiles are not callable
    /// by CPI), so the ed25519 instruction at `ed25519_ix_index` verifies it,
    /// and the program checks that instruction's key and message. The field
    /// exists so that the proven transaction carries the signature, as the
    /// EVM forwarder's wrap input carries its Permit2 signature: a submitter
    /// reads it from here ([`settlement_calls`](crate::settlement_calls)) to
    /// build that ed25519 instruction, and needs nothing but the transaction.
    /// Do not remove it as unused.
    pub signature: [u8; 64],
    pub ed25519_ix_index: u8,
}

impl WrapInput {
    /// The message the user signs for this wrap through the forwarder
    /// `forwarder_id`, as the forwarder recomputes it.
    pub fn to_message(&self, forwarder_id: &[u8; 32]) -> WrapMessage {
        WrapMessage {
            forwarder_id: *forwarder_id,
            token_mint: self.token_mint,
            amount: self.amount,
            nonce: self.nonce,
            deadline: self.deadline,
            action_tree_root: self.action_tree_root,
        }
    }
}

/// An unwrap's input: the forwarder releases `amount` of `token_mint` from
/// escrow to `recipient`'s token account.
#[derive(Clone, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub struct UnwrapInput {
    pub token_mint: [u8; 32],
    pub amount: u64,
    pub recipient: [u8; 32],
}

/// The instruction data is not a wrap's or an unwrap's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    /// The op byte is none of `OP_WRAP`, `OP_UNWRAP` and `OP_MIGRATE`, or
    /// there is none.
    UnknownOp(Option<u8>),
    /// The input after the op byte has the wrong length for its op.
    Length { op: u8, len: usize },
}

impl core::fmt::Display for InputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InputError::UnknownOp(Some(op)) => write!(f, "unknown forwarder op {op}"),
            InputError::UnknownOp(None) => write!(f, "empty forwarder instruction data"),
            InputError::Length { op, len } => {
                write!(f, "forwarder op {op} with a {len}-byte input")
            }
        }
    }
}

impl std::error::Error for InputError {}

/// Decode a forwarder call's instruction data, as the forwarder parses it.
pub fn decode_forwarder_input(data: &[u8]) -> Result<ForwarderInput, InputError> {
    let (&op, input) = data.split_first().ok_or(InputError::UnknownOp(None))?;
    let decoded = match op {
        OP_WRAP => WrapInput::try_from_slice(input).map(ForwarderInput::Wrap),
        OP_UNWRAP => UnwrapInput::try_from_slice(input).map(ForwarderInput::Unwrap),
        OP_MIGRATE => MigrateInput::try_from_slice(input).map(ForwarderInput::Migrate),
        op => return Err(InputError::UnknownOp(Some(op))),
    };
    // Every field is fixed-size or a length-prefixed list of fixed-size
    // entries: Borsh refuses only too few or too many bytes.
    decoded.map_err(|_| InputError::Length {
        op,
        len: input.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap_input() -> WrapInput {
        WrapInput {
            token_mint: [1; 32],
            amount: 42,
            user: [2; 32],
            nonce: 7,
            deadline: -5,
            action_tree_root: [3; 32],
            signature: [4; 64],
            ed25519_ix_index: 2,
        }
    }

    #[test]
    fn op_wrap_is_the_idl_constant_and_the_encoded_op_byte() {
        use crate::generated::spl_token_forwarder::constants::OP_WRAP;
        assert_eq!(OP_WRAP, 0, "the IDL's OP_WRAP is the wire value 0");
        assert_eq!(encode_wrap_forwarder_input(&wrap_input())[0], OP_WRAP);
    }

    #[test]
    fn wrap_input_layout_is_186_bytes() {
        let bytes = encode_wrap_forwarder_input(&wrap_input());
        // Pin the wire layout field by field: the forwarder's `WrapInput`
        // parses exactly these bytes after the op byte, and the resource
        // circuit commits them, so a silent field change must fail here.
        assert_eq!(bytes.len(), 186);
        assert_eq!(bytes[0], OP_WRAP);
        assert_eq!(&bytes[1..33], &[1u8; 32], "token_mint");
        assert_eq!(
            u64::from_le_bytes(bytes[33..41].try_into().unwrap()),
            42,
            "amount (LE)"
        );
        assert_eq!(&bytes[41..73], &[2u8; 32], "user");
        assert_eq!(
            u64::from_le_bytes(bytes[73..81].try_into().unwrap()),
            7,
            "nonce (LE)"
        );
        assert_eq!(
            i64::from_le_bytes(bytes[81..89].try_into().unwrap()),
            -5,
            "deadline (LE)"
        );
        assert_eq!(&bytes[89..121], &[3u8; 32], "action_tree_root");
        assert_eq!(&bytes[121..185], &[4u8; 64], "signature");
        assert_eq!(bytes[185], 2, "ed25519_ix_index");
    }

    #[test]
    fn unwrap_input_length_is_73_bytes() {
        let bytes = encode_unwrap_forwarder_input(&[1u8; 32], 100, &[2u8; 32]);
        assert_eq!(bytes.len(), 73);
        assert_eq!(bytes[0], OP_UNWRAP);
    }

    #[test]
    fn the_decoder_reads_what_the_encoders_write() {
        let wrap = encode_wrap_forwarder_input(&wrap_input());
        assert_eq!(
            decode_forwarder_input(&wrap),
            Ok(ForwarderInput::Wrap(wrap_input()))
        );
        let unwrap = encode_unwrap_forwarder_input(&[1; 32], 100, &[4; 32]);
        assert_eq!(
            decode_forwarder_input(&unwrap),
            Ok(ForwarderInput::Unwrap(UnwrapInput {
                token_mint: [1; 32],
                amount: 100,
                recipient: [4; 32],
            }))
        );
        assert_eq!(
            decode_forwarder_input(&wrap[..185]),
            Err(InputError::Length {
                op: OP_WRAP,
                len: 184
            })
        );
        assert_eq!(
            decode_forwarder_input(&[9]),
            Err(InputError::UnknownOp(Some(9)))
        );
        assert_eq!(
            decode_forwarder_input(&[]),
            Err(InputError::UnknownOp(None))
        );
    }

    fn migrate_input(entries: usize) -> MigrateInput {
        MigrateInput {
            token_mint: [1; 32],
            quantity: 300,
            entries: (0..entries as u8)
                .map(|i| MigrateEntry {
                    nullifier: [10 + i; 32],
                    commitment_tree_root: [20 + i; 32],
                    vulnerable_logic_ref: [30 + i; 32],
                    forwarder: [40 + i; 32],
                })
                .collect(),
        }
    }

    #[test]
    fn migrate_input_layout_is_the_header_then_128_bytes_per_entry() {
        let bytes = encode_migrate_forwarder_input(&migrate_input(2));
        // Pin the wire layout field by field: the emergency-migrating
        // build's Borsh `MigrateInput` parses these bytes after the op byte,
        // and the trigger's logic commits them.
        assert_eq!(bytes.len(), 1 + 32 + 8 + 4 + 2 * 128);
        assert_eq!(bytes[0], OP_MIGRATE);
        assert_eq!(&bytes[1..33], &[1u8; 32], "token_mint");
        assert_eq!(
            u64::from_le_bytes(bytes[33..41].try_into().unwrap()),
            300,
            "quantity (LE)"
        );
        assert_eq!(
            u32::from_le_bytes(bytes[41..45].try_into().unwrap()),
            2,
            "entry count (LE)"
        );
        assert_eq!(&bytes[45..77], &[10u8; 32], "first nullifier");
        assert_eq!(&bytes[77..109], &[20u8; 32], "first root");
        assert_eq!(&bytes[109..141], &[30u8; 32], "first logic ref");
        assert_eq!(&bytes[141..173], &[40u8; 32], "first forwarder");
        assert_eq!(&bytes[173..205], &[11u8; 32], "second nullifier");
    }

    #[test]
    fn the_decoder_reads_a_migration_of_exactly_its_entries() {
        for entries in [0, 1, 3] {
            let bytes = encode_migrate_forwarder_input(&migrate_input(entries));
            assert_eq!(
                decode_forwarder_input(&bytes),
                Ok(ForwarderInput::Migrate(migrate_input(entries)))
            );
        }
        let bytes = encode_migrate_forwarder_input(&migrate_input(2));
        for wrong in [
            &bytes[..bytes.len() - 1],
            &[bytes.as_slice(), &[0]].concat(),
        ] {
            assert_eq!(
                decode_forwarder_input(wrong),
                Err(InputError::Length {
                    op: OP_MIGRATE,
                    len: wrong.len() - 1
                })
            );
        }
    }
}
