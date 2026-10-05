//! The forwarder's instruction data: the op byte and the wrap or unwrap
//! input, which a resource's external call carries as its instruction data.

use anoma_pa_solana_client::cursor::{Cursor, Truncated};

/// Op byte prepended to `WrapInput`-shaped instruction data.
pub const OP_WRAP: u8 = 0;

/// Op byte prepended to `UnwrapInput`-shaped instruction data.
pub const OP_UNWRAP: u8 = 1;

/// Build the 122-byte forwarder instruction data for a wrap.
///
/// Layout: `op(1) + token_mint(32) + amount_le(8) + user(32) + nonce_le(8) +
/// deadline_le_i64(8) + action_tree_root(32) + ed25519_ix_index(1)`.
///
/// The user's ed25519 signature is not part of the input: it reaches the chain
/// in the ed25519 program instruction at `ed25519_ix_index`, which the
/// forwarder verifies through the instructions sysvar.
///
/// `deadline` is signed i64 to match the forwarder's `WrapInput::deadline: i64`.
pub fn encode_wrap_forwarder_input(
    token_mint: &[u8; 32],
    amount: u64,
    user: &[u8; 32],
    nonce: u64,
    deadline: i64,
    action_tree_root: &[u8; 32],
    ed25519_ix_index: u8,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(122);
    buf.push(OP_WRAP);
    buf.extend_from_slice(token_mint);
    buf.extend_from_slice(&amount.to_le_bytes());
    buf.extend_from_slice(user);
    buf.extend_from_slice(&nonce.to_le_bytes());
    buf.extend_from_slice(&deadline.to_le_bytes());
    buf.extend_from_slice(action_tree_root);
    buf.push(ed25519_ix_index);
    buf
}

/// Build the 73-byte forwarder instruction data for an unwrap.
///
/// Layout: `op(1) + token_mint(32) + amount_le(8) + recipient(32)`.
pub fn encode_unwrap_forwarder_input(
    token_mint: &[u8; 32],
    amount: u64,
    recipient: &[u8; 32],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(73);
    buf.push(OP_UNWRAP);
    buf.extend_from_slice(token_mint);
    buf.extend_from_slice(&amount.to_le_bytes());
    buf.extend_from_slice(recipient);
    buf
}

/// A forwarder call's instruction data, decoded: what a submitter reads from
/// the call a proof commits to supply its accounts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwarderInput {
    Wrap(WrapInput),
    Unwrap(UnwrapInput),
}

/// A wrap's input: the forwarder escrows `amount` of `token_mint` from
/// `user`'s token account, authorized by the user's ed25519 signature, which
/// the settlement transaction carries at `ed25519_ix_index`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrapInput {
    pub token_mint: [u8; 32],
    pub amount: u64,
    pub user: [u8; 32],
    pub nonce: u64,
    pub deadline: i64,
    pub action_tree_root: [u8; 32],
    pub ed25519_ix_index: u8,
}

/// An unwrap's input: the forwarder releases `amount` of `token_mint` from
/// escrow to `recipient`'s token account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnwrapInput {
    pub token_mint: [u8; 32],
    pub amount: u64,
    pub recipient: [u8; 32],
}

/// The instruction data is not a wrap's or an unwrap's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    /// The op byte is neither `OP_WRAP` nor `OP_UNWRAP`, or there is none.
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

fn read_wrap(input: &mut Cursor) -> Result<WrapInput, Truncated> {
    Ok(WrapInput {
        token_mint: input.array_32("token_mint")?,
        amount: input.u64_le("amount")?,
        user: input.array_32("user")?,
        nonce: input.u64_le("nonce")?,
        // The i64's little-endian bytes, read as a u64 and reinterpreted.
        deadline: input.u64_le("deadline")? as i64,
        action_tree_root: input.array_32("action_tree_root")?,
        ed25519_ix_index: input.u8("ed25519_ix_index")?,
    })
}

fn read_unwrap(input: &mut Cursor) -> Result<UnwrapInput, Truncated> {
    Ok(UnwrapInput {
        token_mint: input.array_32("token_mint")?,
        amount: input.u64_le("amount")?,
        recipient: input.array_32("recipient")?,
    })
}

/// Decode a forwarder call's instruction data, as the forwarder parses it.
pub fn decode_forwarder_input(data: &[u8]) -> Result<ForwarderInput, InputError> {
    let (&op, input) = data.split_first().ok_or(InputError::UnknownOp(None))?;
    let mut cursor = Cursor::new(input, 0);
    match op {
        OP_WRAP if input.len() == 121 => Ok(ForwarderInput::Wrap(
            read_wrap(&mut cursor).expect("a 121-byte input holds every wrap field"),
        )),
        OP_UNWRAP if input.len() == 72 => Ok(ForwarderInput::Unwrap(
            read_unwrap(&mut cursor).expect("a 72-byte input holds every unwrap field"),
        )),
        OP_WRAP | OP_UNWRAP => Err(InputError::Length {
            op,
            len: input.len(),
        }),
        op => Err(InputError::UnknownOp(Some(op))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_input_layout_is_122_bytes() {
        let bytes = encode_wrap_forwarder_input(
            &[1u8; 32],
            42,
            &[2u8; 32],
            7,
            1_700_000_000,
            &[3u8; 32],
            2,
        );
        // Pin the wire layout field by field: the forwarder's `WrapInput`
        // parses exactly these bytes after the op byte, and the resource
        // circuit commits them, so a silent field change must fail here.
        assert_eq!(bytes.len(), 122);
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
            1_700_000_000,
            "deadline (LE)"
        );
        assert_eq!(&bytes[89..121], &[3u8; 32], "action_tree_root");
        assert_eq!(bytes[121], 2, "ed25519_ix_index");
    }

    #[test]
    fn unwrap_input_length_is_73_bytes() {
        let bytes = encode_unwrap_forwarder_input(&[1u8; 32], 100, &[2u8; 32]);
        assert_eq!(bytes.len(), 73);
        assert_eq!(bytes[0], OP_UNWRAP);
    }

    #[test]
    fn the_decoder_reads_what_the_encoders_write() {
        let wrap = encode_wrap_forwarder_input(&[1; 32], 42, &[2; 32], 7, -5, &[3; 32], 2);
        assert_eq!(
            decode_forwarder_input(&wrap),
            Ok(ForwarderInput::Wrap(WrapInput {
                token_mint: [1; 32],
                amount: 42,
                user: [2; 32],
                nonce: 7,
                deadline: -5,
                action_tree_root: [3; 32],
                ed25519_ix_index: 2,
            }))
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
            decode_forwarder_input(&wrap[..121]),
            Err(InputError::Length {
                op: OP_WRAP,
                len: 120
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
}
