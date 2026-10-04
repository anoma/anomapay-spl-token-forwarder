//! The forwarder's instruction data: the op byte and the wrap or unwrap
//! input, which a resource's external call carries as its instruction data.

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
    token_mint: &[u8],
    amount: u64,
    user: &[u8],
    nonce: u64,
    deadline: i64,
    action_tree_root: &[u8],
    ed25519_ix_index: u8,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(122);
    buf.push(OP_WRAP);
    buf.extend_from_slice(&pad_to_32(token_mint));
    buf.extend_from_slice(&amount.to_le_bytes());
    buf.extend_from_slice(&pad_to_32(user));
    buf.extend_from_slice(&nonce.to_le_bytes());
    buf.extend_from_slice(&deadline.to_le_bytes());
    buf.extend_from_slice(&pad_to_32(action_tree_root));
    buf.push(ed25519_ix_index);
    buf
}

/// Build the 73-byte forwarder instruction data for an unwrap.
///
/// Layout: `op(1) + token_mint(32) + amount_le(8) + recipient(32)`.
pub fn encode_unwrap_forwarder_input(token_mint: &[u8], amount: u64, recipient: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(73);
    buf.push(OP_UNWRAP);
    buf.extend_from_slice(&pad_to_32(token_mint));
    buf.extend_from_slice(&amount.to_le_bytes());
    buf.extend_from_slice(&pad_to_32(recipient));
    buf
}

fn pad_to_32(input: &[u8]) -> [u8; 32] {
    assert!(
        input.len() <= 32,
        "input too long for 32-byte field: {} bytes",
        input.len()
    );
    let mut out = [0u8; 32];
    out[..input.len()].copy_from_slice(input);
    out
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

/// Decode a forwarder call's instruction data, as the forwarder parses it.
pub fn decode_forwarder_input(data: &[u8]) -> Result<ForwarderInput, InputError> {
    let (&op, input) = data.split_first().ok_or(InputError::UnknownOp(None))?;
    let length = InputError::Length {
        op,
        len: input.len(),
    };
    let field32 = |at: usize| -> [u8; 32] { input[at..at + 32].try_into().expect("32 bytes") };
    let field8 = |at: usize| -> [u8; 8] { input[at..at + 8].try_into().expect("8 bytes") };
    match op {
        OP_WRAP if input.len() == 121 => Ok(ForwarderInput::Wrap(WrapInput {
            token_mint: field32(0),
            amount: u64::from_le_bytes(field8(32)),
            user: field32(40),
            nonce: u64::from_le_bytes(field8(72)),
            deadline: i64::from_le_bytes(field8(80)),
            action_tree_root: field32(88),
            ed25519_ix_index: input[120],
        })),
        OP_UNWRAP if input.len() == 72 => Ok(ForwarderInput::Unwrap(UnwrapInput {
            token_mint: field32(0),
            amount: u64::from_le_bytes(field8(32)),
            recipient: field32(40),
        })),
        OP_WRAP | OP_UNWRAP => Err(length),
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
