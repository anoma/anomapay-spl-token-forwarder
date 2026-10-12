//! The forwarder's instruction data: the op byte and the wrap or unwrap
//! input, which a resource's external call carries as its instruction data.

use anoma_pa_solana_client::cursor::{Cursor, Truncated};

/// Op byte prepended to `WrapInput`-shaped instruction data.
pub const OP_WRAP: u8 = 0;

/// Op byte prepended to `UnwrapInput`-shaped instruction data.
pub const OP_UNWRAP: u8 = 1;

/// Build the 186-byte forwarder instruction data for the wrap `input`.
///
/// Layout: `op(1) + token_mint(32) + amount_le(8) + user(32) + nonce_le(8) +
/// deadline_le_i64(8) + action_tree_root(32) + signature(64) +
/// ed25519_ix_index(1)`.
pub fn encode_wrap_forwarder_input(input: &WrapInput) -> Vec<u8> {
    let mut buf = Vec::with_capacity(186);
    buf.push(OP_WRAP);
    buf.extend_from_slice(&input.token_mint);
    buf.extend_from_slice(&input.amount.to_le_bytes());
    buf.extend_from_slice(&input.user);
    buf.extend_from_slice(&input.nonce.to_le_bytes());
    buf.extend_from_slice(&input.deadline.to_le_bytes());
    buf.extend_from_slice(&input.action_tree_root);
    buf.extend_from_slice(&input.signature);
    buf.push(input.ed25519_ix_index);
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
/// the settlement transaction verifies in the ed25519 instruction at
/// `ed25519_ix_index`.
#[derive(Clone, Debug, PartialEq, Eq)]
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
        signature: input
            .take(64, "signature")?
            .try_into()
            .expect("take returns the 64 bytes it was asked for"),
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
        OP_WRAP if input.len() == 185 => Ok(ForwarderInput::Wrap(
            read_wrap(&mut cursor).expect("a 185-byte input holds every wrap field"),
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
}
