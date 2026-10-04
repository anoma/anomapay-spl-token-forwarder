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
}
