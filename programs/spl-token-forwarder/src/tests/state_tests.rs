//! Tests for the wrap and unwrap inputs and the wrap message.

use crate::state::{UnwrapInput, WrapInput, WrapMessage};
use anchor_lang::prelude::{borsh, Pubkey};

/// The field-by-field hash is sha256 of the message's 120-byte Borsh encoding.
#[test]
fn wrap_message_hash_is_sha256_of_its_borsh_encoding() {
    let msg = WrapMessage {
        forwarder_id: [0xAAu8; 32],
        token_mint: [1u8; 32],
        amount: 1000,
        nonce: 42,
        deadline: -1_700_000_000,
        action_tree_root: [2u8; 32],
    };
    let encoded = borsh::to_vec(&msg).unwrap();
    assert_eq!(encoded.len(), WrapMessage::SIZE);
    assert_eq!(msg.hash(), solana_sha256_hasher::hash(&encoded).to_bytes());
}

#[test]
fn test_wrap_message_domain_separation() {
    // Same message with different forwarder IDs should produce different hashes
    let msg1 = WrapMessage {
        forwarder_id: [1u8; 32],
        token_mint: [0u8; 32],
        amount: 100,
        nonce: 1,
        deadline: 1700000000,
        action_tree_root: [0u8; 32],
    };

    let msg2 = WrapMessage {
        forwarder_id: [2u8; 32], // Different forwarder
        token_mint: [0u8; 32],
        amount: 100,
        nonce: 1,
        deadline: 1700000000,
        action_tree_root: [0u8; 32],
    };

    // Hashes must be different due to domain separation
    assert_ne!(msg1.hash(), msg2.hash());
}

#[test]
fn unwrap_input_round_trips_through_to_bytes() {
    let input = UnwrapInput {
        token_mint: Pubkey::new_unique(),
        amount: 50_000_000,
        recipient: Pubkey::new_unique(),
    };
    let bytes = input.to_bytes();
    assert_eq!(bytes.len(), UnwrapInput::SIZE);
    let parsed = UnwrapInput::try_from_bytes(&bytes).unwrap();
    assert_eq!(parsed.token_mint, input.token_mint);
    assert_eq!(parsed.amount, input.amount);
    assert_eq!(parsed.recipient, input.recipient);
}

#[test]
fn wrap_input_rejects_any_other_length() {
    for len in [WrapInput::SIZE - 1, WrapInput::SIZE + 1] {
        assert_eq!(
            WrapInput::try_from_bytes(&vec![0u8; len]).unwrap_err(),
            crate::ErrorCode::InvalidInputLength.into(),
            "a {len}-byte wrap input"
        );
    }
    assert!(WrapInput::try_from_bytes(&[0u8; WrapInput::SIZE]).is_ok());
}

#[test]
fn unwrap_input_rejects_any_other_length() {
    for len in [UnwrapInput::SIZE - 1, UnwrapInput::SIZE + 1] {
        assert_eq!(
            UnwrapInput::try_from_bytes(&vec![0u8; len]).unwrap_err(),
            crate::ErrorCode::InvalidInputLength.into(),
            "a {len}-byte unwrap input"
        );
    }
    assert!(UnwrapInput::try_from_bytes(&[0u8; UnwrapInput::SIZE]).is_ok());
}
