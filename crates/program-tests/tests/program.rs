//! This crate against the program: the inputs it encodes are the ones the
//! program parses, and the addresses and sizes it derives are the program's.

use anchor_lang::prelude::Pubkey;
use spl_token_forwarder::state::{NonceBitmap, UnwrapInput, WrapInput, NONCE_BITMAP_SEED};

/// The client library encodes the wrap input this program parses, and the
/// message it recomputes from that input is the one the client tells the
/// wallet to sign.
#[test]
fn wrap_input_parses_the_client_encoding_and_recomputes_its_signed_message() {
    let (mint, user, root, signature) = ([1u8; 32], [2u8; 32], [3u8; 32], [4u8; 64]);
    let encoded = anomapay_spl_token_forwarder_client::encode_wrap_forwarder_input(
        &anomapay_spl_token_forwarder_client::WrapInput {
            token_mint: mint,
            amount: 1000,
            user,
            nonce: 42,
            deadline: 1_700_000_000,
            action_tree_root: root,
            signature,
            ed25519_ix_index: 3,
        },
    );
    let (op, operand) = encoded.split_first().unwrap();
    assert_eq!(*op, spl_token_forwarder::OP_WRAP);
    let input = WrapInput::try_from_bytes(operand).unwrap();
    assert_eq!(input.token_mint.to_bytes(), mint);
    assert_eq!(input.amount, 1000);
    assert_eq!(input.user.to_bytes(), user);
    assert_eq!(input.nonce, 42);
    assert_eq!(input.deadline, 1_700_000_000);
    assert_eq!(input.action_tree_root, root);
    assert_eq!(input.signature, signature);
    assert_eq!(input.ed25519_ix_index, 3);

    let client_message = anomapay_spl_token_forwarder_client::WrapMessage {
        forwarder_id: spl_token_forwarder::ID.to_bytes(),
        token_mint: mint,
        amount: 1000,
        nonce: 42,
        deadline: 1_700_000_000,
        action_tree_root: root,
    };
    assert_eq!(
        input.to_message(&spl_token_forwarder::ID).signed_message(),
        client_message.base64_digest().as_bytes()
    );
}

/// The client library encodes the unwrap input this program parses.
#[test]
fn unwrap_input_parses_the_client_encoding() {
    let (mint, recipient) = ([4u8; 32], [5u8; 32]);
    let encoded =
        anomapay_spl_token_forwarder_client::encode_unwrap_forwarder_input(&mint, 77, &recipient);
    let (op, operand) = encoded.split_first().unwrap();
    assert_eq!(*op, spl_token_forwarder::OP_UNWRAP);
    let input = UnwrapInput::try_from_bytes(operand).unwrap();
    assert_eq!(input.token_mint.to_bytes(), mint);
    assert_eq!(input.amount, 77);
    assert_eq!(input.recipient.to_bytes(), recipient);
}

/// The client derives a user's bitmap address from the program's own seed.
/// The IDL cannot check it: its seeds name the args `user` and
/// `word_index`, which it lists as the handler's `_user` and `_word_index`.
#[test]
fn the_clients_nonce_bitmap_address_is_the_programs() {
    for (user, word_index) in [([7u8; 32], 0), ([8u8; 32], 3), ([9u8; 32], u64::MAX)] {
        let user = Pubkey::new_from_array(user);
        let client = anomapay_spl_token_forwarder_client::derive_nonce_bitmap_pda(
            &spl_token_forwarder::ID,
            &user,
            word_index,
        );
        let program = Pubkey::find_program_address(
            &[NONCE_BITMAP_SEED, user.as_ref(), &word_index.to_le_bytes()],
            &spl_token_forwarder::ID,
        );
        println!("({user}, word {word_index}): {client:?}");
        assert_eq!(client, program, "({user}, word {word_index})");
    }
}

#[test]
fn nonce_bitmap_account_is_discriminator_word_and_bump() {
    assert_eq!(NonceBitmap::ACCOUNT_SIZE, 8 + 32 + 1);
    assert_eq!(
        NonceBitmap::ACCOUNT_SIZE,
        anomapay_spl_token_forwarder_client::constants::NONCE_BITMAP_ACCOUNT_SIZE,
        "the client's size is the program's"
    );
}
