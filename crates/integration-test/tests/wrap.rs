//! Wraps through the forwarder on the local environment: a settled wrap, its
//! events and the nonce it uses up, and every account, signature and replay
//! the forwarder refuses, with no tokens moved.

use anoma_pa_solana_client::events::{PaEvent, decode_event_instruction};
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_pa_solana_integration_test::test_forwarder;
use anoma_pa_testkit::transaction::Transaction;
use anomapay_spl_token_forwarder_client::{
    ForwarderEvent, build_wrap_forwarder_accounts, decode_forwarder_event_instruction,
    decode_nonce_bitmap, derive_associated_token_address, derive_forwarder_escrow_authority,
    derive_nonce_bitmap_pda, init_nonce_bitmap_ix, nonce_word_index,
};
use anomapay_spl_token_forwarder_integration_test::fixtures::ShieldedOwner;
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::refusal::{balances, refuses};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, Forwarder, LocalForwarder, create_mint, fund, token_account,
};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;
/// The forwarder nonce of every test's first wrap.
const NONCE: u64 = 1;

/// Where a wrap segment (`build_wrap_forwarder_accounts`) passes the config,
/// the user's token account, the escrow's and the nonce bitmap.
const CONFIG: usize = 1;
const SOURCE: usize = 5;
const DESTINATION: usize = 6;
const NONCE_BITMAP: usize = 8;

/// The local environment with the forwarder, and the owner of the resources
/// its wraps create.
async fn local() -> anyhow::Result<(LocalEnv, LocalForwarder, ShieldedOwner)> {
    let (env, local) = setup::local().await?;
    Ok((env, local, ShieldedOwner::seeded("wrap/owner")))
}

/// The first wrap of the user: the submitter passes the user's signature
/// check, then creates the nonce bitmap the wrap needs.
async fn first_wrap(
    env: &LocalEnv,
    forwarder: &Forwarder,
    owner: &ShieldedOwner,
) -> anyhow::Result<setup::ProvenWrap> {
    forwarder
        .prove_wrap(env, owner, AMOUNT, NONCE, "wrap/first")
        .await
}

// Mirrors ERC20Forwarder.t.sol: test_wrap_pulls_funds_from_user. The first
// wrap on a word carries init_nonce_bitmap in the same transaction, after
// the ed25519 instruction the wrap input points at (index 0).
#[tokio::test(flavor = "multi_thread")]
async fn settles_a_wrap_the_escrow_receives_the_tokens_and_the_nonce_is_used() -> anyhow::Result<()>
{
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let (user, escrow) = (forwarder.user_account(), forwarder.escrow_account());
    let [user_before, escrow_before] = balances(&env, &[user, escrow]).await?[..] else {
        unreachable!("two accounts, two balances");
    };

    // An instruction before the settlement uses up the transaction's
    // program-log budget (Agave keeps 10,000 bytes, counting each line with
    // its "Program log: " prefix; the test forwarder logs 100-byte lines), so
    // the runtime truncates the settlement's log.
    let test_forwarder = env.deploy_test_forwarder()?;
    let lines = 10_000usize.div_ceil("Program log: ".len() + 100) as u8;
    forwarder.rewrite(&mut env, move |accounts| {
        accounts
            .preceding
            .push(test_forwarder::log_ix(&test_forwarder, lines))
    });
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let signature = env.protocol_adapter.settle(wrap.tx).await?;
    forwarder.restore(&mut env);

    anyhow::ensure!(
        balances(&env, &[user, escrow]).await? == [user_before - AMOUNT, escrow_before + AMOUNT],
        "the wrap moves {AMOUNT} from the user's account to the escrow's"
    );

    // Mirrors ERC20Forwarder's `Wrapped` event. It is a CPI event, part of
    // the transaction, so the truncated log cannot drop it.
    let executed = Executed::read(&env.protocol_adapter.rpc, &signature).await?;
    anyhow::ensure!(
        executed.logs.iter().any(|line| line == "Log truncated"),
        "the settlement's log is not truncated: {:?}",
        executed.logs
    );
    let wrapped: Vec<_> = executed
        .cpi_events(&forwarder.program)
        .map(decode_forwarder_event_instruction)
        .collect::<Result<_, _>>()?;
    let [ForwarderEvent::Wrapped(event)] = &wrapped[..] else {
        anyhow::bail!("the settlement emits {wrapped:?}, not one Wrapped event");
    };
    anyhow::ensure!(
        event.token_mint == forwarder.mint.to_bytes()
            && event.from == forwarder.user.pubkey().to_bytes()
            && event.amount == AMOUNT
            && event.nonce == NONCE,
        "the Wrapped event {event:?} is not the wrap of {AMOUNT} with nonce {NONCE}"
    );

    // Both resources carry the AnomaPay transfer logic the forwarder config
    // pins: the wrap settled under the real verifying key.
    let actions: Vec<_> = executed
        .cpi_events(&env.protocol_adapter.program)
        .map(decode_event_instruction)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|event| match event {
            PaEvent::ActionExecuted(action) => Some(action),
            _ => None,
        })
        .collect();
    let [action] = &actions[..] else {
        anyhow::bail!("the settlement executes {} actions, not one", actions.len());
    };
    let transfer: [u8; 32] = logic_ref().into();
    anyhow::ensure!(
        action.consumed_logic_refs == [transfer] && action.created_logic_refs == [transfer],
        "the action's resources carry the logic refs {:02x?} and {:02x?}, not the transfer \
         logic's",
        action.consumed_logic_refs,
        action.created_logic_refs
    );

    let (bitmap, bump) = derive_nonce_bitmap_pda(
        &forwarder.program,
        &forwarder.user.pubkey(),
        nonce_word_index(NONCE),
    );
    let bitmap = decode_nonce_bitmap(&env.protocol_adapter.rpc.get_account_data(&bitmap).await?)?;
    anyhow::ensure!(bitmap.is_used(NONCE), "the wrap's nonce is not marked used");
    anyhow::ensure!(
        bitmap.bump == bump,
        "init_nonce_bitmap stores the bump {}, not the canonical {bump}",
        bitmap.bump
    );
    Ok(())
}

// The largest settlement the forwarder takes part in: the ed25519
// authorization, the inline bitmap init and the settle with the wrap
// segment. Sent as a v0 transaction through the settlement lookup table, it
// fits one packet, and every account of the forwarder's that is the same for
// every wrap of the mint is looked up rather than static: the only static
// ones are signers, invoked programs and the user's own accounts.
#[tokio::test(flavor = "multi_thread")]
async fn the_first_wrap_fits_one_packet_with_the_forwarders_fixed_accounts_looked_up()
-> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let signature = env.protocol_adapter.settle(wrap.tx).await?;
    let executed = Executed::read(&env.protocol_adapter.rpc, &signature).await?;

    let size = bincode::serialize(&executed.transaction)?.len();
    let message = &executed.transaction.message;
    println!(
        "first-wrap settlement as v0: {size} bytes, {} static keys, {} looked up",
        message.static_account_keys().len(),
        executed.loaded.len()
    );
    anyhow::ensure!(
        size <= solana_packet::PACKET_DATA_SIZE,
        "the first-wrap settlement is {size} bytes, more than one packet"
    );

    let user = forwarder.user.pubkey();
    let payer = env.protocol_adapter.payer.pubkey();
    let per_user = [
        forwarder.user_account(),
        derive_nonce_bitmap_pda(&forwarder.program, &user, nonce_word_index(NONCE)).0,
        payer,
    ];
    let invoked: Vec<Pubkey> = message
        .instructions()
        .iter()
        .map(|ix| message.static_account_keys()[usize::from(ix.program_id_index)])
        .collect();
    let forwarders_accounts =
        build_wrap_forwarder_accounts(&forwarder.program, &user, &forwarder.mint, NONCE)
            .into_iter()
            .chain(
                init_nonce_bitmap_ix(&forwarder.program, &payer, &user, nonce_word_index(NONCE))
                    .accounts,
            )
            .map(|meta| meta.pubkey);
    for key in forwarders_accounts {
        if per_user.contains(&key) || invoked.contains(&key) {
            continue;
        }
        anyhow::ensure!(
            executed.loaded.contains(&key),
            "the forwarder's fixed account {key} is static, not looked up: the settlement \
             lookup table lacks it"
        );
    }
    Ok(())
}

// The adapter forwards no signer to the forwarder, so the forwarder cannot
// create the bitmap during the wrap; a wrap on a word without one fails.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_whose_nonce_bitmap_does_not_exist() -> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let program = forwarder.program;
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        move |accounts| accounts.preceding.retain(|ix| ix.program_id != program),
        "Error Code: NonceBitmapMissing.",
        &unmoved,
    )
    .await
}

// The destination account is chosen by the submitter, not by the proof.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_whose_destination_the_escrow_does_not_own() -> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let other = token_account(&env, &Keypair::new().pubkey(), &forwarder.mint).await?;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let unmoved = [forwarder.user_account(), forwarder.escrow_account(), other];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        move |accounts| accounts.segment[DESTINATION].pubkey = other,
        "Error Code: WrongTokenAccountOwner.",
        &unmoved,
    )
    .await
}

// The source account is chosen by the submitter, not by the proof. An
// account whose owner approved the escrow as delegate must not fund a wrap
// someone else signed: the wrap debits only the signing user.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_whose_source_the_signing_user_does_not_own() -> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let other = fund(
        &env,
        &forwarder.program,
        &forwarder.mint,
        &Keypair::new(),
        AMOUNT,
    )
    .await?;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let unmoved = [other, forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        move |accounts| accounts.segment[SOURCE].pubkey = other,
        "Error Code: WrongTokenAccountOwner.",
        &unmoved,
    )
    .await
}

// Every transfer names its mint only through the input; SPL Transfer checks
// only that source and destination share a mint. A wrap of this mint must
// not move tokens of another one.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_that_moves_tokens_of_another_mint() -> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let other_mint = create_mint(&env, &forwarder.program).await?;
    let user_other = fund(
        &env,
        &forwarder.program,
        &other_mint,
        &forwarder.user,
        AMOUNT,
    )
    .await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&forwarder.program);
    let escrow_other = derive_associated_token_address(&escrow_authority, &other_mint);
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let unmoved = [user_other, escrow_other, forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        move |accounts| {
            accounts.segment[SOURCE].pubkey = user_other;
            accounts.segment[DESTINATION].pubkey = escrow_other;
        },
        "Error Code: WrongTokenAccountMint.",
        &unmoved,
    )
    .await
}

// The wrap input names the user and the index of the ed25519 instruction that
// authorizes it; the submitter supplies that instruction. Each of these
// reaches the authorization check (the bitmap exists, the accounts are the
// user's and the escrow's) and must stop there with no tokens moved.

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_authorized_by_another_keys_signature_over_the_wrap_message()
-> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let other = Keypair::new();
    let signature = other.sign_message(&wrap.authorization.message);
    let ed25519 = solana_ed25519_program::new_ed25519_instruction_with_signature(
        &wrap.authorization.message,
        &<[u8; 64]>::from(signature),
        &other.pubkey().to_bytes(),
    );
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        move |accounts| accounts.preceding[0] = ed25519.clone(),
        "Error Code: Ed25519PubkeyMismatch.",
        &unmoved,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_whose_authorization_is_the_users_signature_over_another_message()
-> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let mut message = wrap.authorization.message.clone();
    message[0] ^= 1;
    let signature = forwarder.user.sign_message(&message);
    let ed25519 = solana_ed25519_program::new_ed25519_instruction_with_signature(
        &message,
        &<[u8; 64]>::from(signature),
        &forwarder.user.pubkey().to_bytes(),
    );
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        move |accounts| accounts.preceding[0] = ed25519.clone(),
        "Error Code: Ed25519MessageMismatch.",
        &unmoved,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_whose_authorization_is_not_at_the_instruction_index_its_input_names()
-> anyhow::Result<()> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    let wrap = first_wrap(&env, forwarder, &owner).await?;
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        wrap.tx,
        // [ed25519, init_nonce_bitmap] becomes [init_nonce_bitmap, ed25519].
        |accounts| accounts.preceding.reverse(),
        "Error Code: InvalidEd25519Instruction.",
        &unmoved,
    )
    .await
}

/// The environment after the user's first wrap settled, and a second wrap
/// under the same nonce, with the user holding and approving its amount, so
/// any account substitution that let it through would move tokens.
async fn replay() -> anyhow::Result<(LocalEnv, LocalForwarder, Transaction)> {
    let (mut env, local, owner) = local().await?;
    let forwarder = &local.forwarder;
    forwarder
        .wrap(&mut env, &owner, AMOUNT, NONCE, "wrap/first")
        .await?;
    fund(
        &env,
        &forwarder.program,
        &forwarder.mint,
        &forwarder.user,
        AMOUNT,
    )
    .await?;
    let replay = forwarder
        .prove_wrap(&env, &owner, AMOUNT, NONCE, "wrap/replay")
        .await?;
    Ok((env, local, replay.tx))
}

// Mirrors ERC20Forwarder.t.sol: test_wrap_reverts_if_the_signature_was_already_used
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_wrap_that_replays_a_used_nonce() -> anyhow::Result<()> {
    let (mut env, local, tx) = replay().await?;
    let forwarder = &local.forwarder;
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        |_| {},
        "Error Code: NonceAlreadyUsed.",
        &unmoved,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_replay_that_supplies_the_users_bitmap_of_another_word() -> anyhow::Result<()> {
    let (mut env, local, tx) = replay().await?;
    let forwarder = &local.forwarder;
    let (program, user) = (forwarder.program, forwarder.user.pubkey());
    let payer = env.protocol_adapter.payer.pubkey();
    let word = nonce_word_index(NONCE) + 1;
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| {
            accounts.segment[NONCE_BITMAP].pubkey =
                derive_nonce_bitmap_pda(&program, &user, word).0;
            accounts
                .preceding
                .push(init_nonce_bitmap_ix(&program, &payer, &user, word));
        },
        "Error Code: InvalidNonceBitmapPda.",
        &unmoved,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_replay_that_supplies_another_users_bitmap_of_the_nonces_word()
-> anyhow::Result<()> {
    let (mut env, local, tx) = replay().await?;
    let forwarder = &local.forwarder;
    let program = forwarder.program;
    let other = Keypair::new().pubkey();
    let payer = env.protocol_adapter.payer.pubkey();
    let word = nonce_word_index(NONCE);
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| {
            accounts.segment[NONCE_BITMAP].pubkey =
                derive_nonce_bitmap_pda(&program, &other, word).0;
            accounts
                .preceding
                .push(init_nonce_bitmap_ix(&program, &payer, &other, word));
        },
        "Error Code: InvalidNonceBitmapPda.",
        &unmoved,
    )
    .await
}

// Not read as an all-zero bitmap in which the nonce is unused.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_replay_whose_nonce_bitmap_is_an_account_of_another_program() -> anyhow::Result<()>
{
    let (mut env, local, tx) = replay().await?;
    let forwarder = &local.forwarder;
    let user_account = forwarder.user_account();
    let unmoved = [user_account, forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| accounts.segment[NONCE_BITMAP].pubkey = user_account,
        "Error Code: NonceBitmapMissing.",
        &unmoved,
    )
    .await
}

// The config the forwarder checks the caller and logic ref against is at a
// fixed address; another account there is refused in account validation.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_forward_call_whose_config_is_an_account_of_another_program() -> anyhow::Result<()>
{
    let (mut env, local, tx) = replay().await?;
    let forwarder = &local.forwarder;
    let mint = forwarder.mint;
    let unmoved = [forwarder.user_account(), forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| accounts.segment[CONFIG].pubkey = mint,
        "AnchorError caused by account: config. Error Code: AccountOwnedByWrongProgram.",
        &unmoved,
    )
    .await
}
