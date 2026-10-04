//! Unwraps through the forwarder on the local environment: a settled unwrap
//! and its event, and every account and recipient the forwarder refuses,
//! with no tokens moved.

use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_rm_risc0::resource::Resource;
use anomapay_spl_token_forwarder_client::{
    ForwarderEvent, decode_forwarder_event_instruction, derive_associated_token_address,
    derive_forwarder_escrow_authority,
};
use anomapay_spl_token_forwarder_integration_test::fixtures::ShieldedOwner;
use anomapay_spl_token_forwarder_integration_test::refusal::{balances, refuses};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, LocalForwarder, balance, create_mint, fund, mint_to, token_account,
};
use solana_keypair::Keypair;
use solana_signer::Signer;

/// 100 tokens at the mint's 6 decimals: what the wrap escrows and the unwrap
/// releases.
const AMOUNT: u64 = 100_000_000;

/// Where an unwrap segment (`build_unwrap_forwarder_accounts`) passes the
/// escrow's token account and the recipient's.
const SOURCE: usize = 5;
const DESTINATION: usize = 6;

/// The local environment after a wrap settled: the forwarder, the wrapped
/// resource, and its owner.
async fn wrapped() -> anyhow::Result<(LocalEnv, LocalForwarder, Resource, ShieldedOwner)> {
    let (mut env, local) = setup::local().await?;
    let owner = ShieldedOwner::seeded("unwrap/owner");
    let wrapped = local
        .forwarder
        .wrap(&mut env, &owner, AMOUNT, 1, "unwrap/wrap")
        .await?;
    Ok((env, local, wrapped, owner))
}

// Mirrors ERC20Forwarder.t.sol: test_unwrap_sends_funds_to_the_user
#[tokio::test(flavor = "multi_thread")]
async fn settles_an_unwrap_the_recipient_receives_the_tokens_from_escrow() -> anyhow::Result<()> {
    let (mut env, local, wrapped, owner) = wrapped().await?;
    let forwarder = &local.forwarder;
    let recipient = Keypair::new().pubkey();
    let escrow = forwarder.escrow_account();
    let escrow_before = balance(&env, &escrow).await?;

    let tx = forwarder
        .prove_unwrap(&env, wrapped, &owner, recipient)
        .await?;
    let signature = env.protocol_adapter.settle(tx).await?;

    let recipient_account = derive_associated_token_address(&recipient, &forwarder.mint);
    anyhow::ensure!(
        balances(&env, &[escrow, recipient_account]).await? == [escrow_before - AMOUNT, AMOUNT],
        "the unwrap moves {AMOUNT} from the escrow's account to the recipient's"
    );

    // Mirrors ERC20Forwarder's `Unwrapped` event, a CPI event like `Wrapped`.
    let executed = Executed::read(&env.protocol_adapter.rpc, &signature).await?;
    let events: Vec<_> = executed
        .cpi_events(&forwarder.program)
        .map(decode_forwarder_event_instruction)
        .collect::<Result<_, _>>()?;
    let [ForwarderEvent::Unwrapped(event)] = &events[..] else {
        anyhow::bail!("the settlement emits {events:?}, not one Unwrapped event");
    };
    anyhow::ensure!(
        event.token_mint == forwarder.mint.to_bytes()
            && event.to == recipient.to_bytes()
            && event.amount == AMOUNT,
        "the Unwrapped event {event:?} is not the release of {AMOUNT} to {recipient}"
    );
    Ok(())
}

// The recipient account is chosen by the submitter, not by the proof.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_unwrap_to_a_token_account_the_recipient_does_not_own() -> anyhow::Result<()> {
    let (mut env, local, wrapped, owner) = wrapped().await?;
    let forwarder = &local.forwarder;
    let tx = forwarder
        .prove_unwrap(&env, wrapped, &owner, Keypair::new().pubkey())
        .await?;
    let user_account = forwarder.user_account();
    let unmoved = [forwarder.escrow_account(), user_account];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| accounts.segment[DESTINATION].pubkey = user_account,
        "Error Code: WrongTokenAccountOwner.",
        &unmoved,
    )
    .await
}

// The escrow authority signs the release; as a delegate it could move any
// account that approved it. An unwrap pays only from an account the escrow
// owns.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_unwrap_whose_source_the_escrow_does_not_own() -> anyhow::Result<()> {
    let (mut env, local, wrapped, owner) = wrapped().await?;
    let forwarder = &local.forwarder;
    let other = fund(
        &env,
        &forwarder.program,
        &forwarder.mint,
        &Keypair::new(),
        AMOUNT,
    )
    .await?;
    let tx = forwarder
        .prove_unwrap(&env, wrapped, &owner, Keypair::new().pubkey())
        .await?;
    let unmoved = [other, forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| accounts.segment[SOURCE].pubkey = other,
        "Error Code: WrongTokenAccountOwner.",
        &unmoved,
    )
    .await
}

// One escrow authority owns every mint's escrow account, so the owner check
// alone does not keep an unwrap to its mint: the input names the mint, and
// the escrow account must hold it.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_unwrap_that_draws_another_mints_escrow() -> anyhow::Result<()> {
    let (mut env, local, wrapped, owner) = wrapped().await?;
    let forwarder = &local.forwarder;
    let recipient = Keypair::new().pubkey();
    let other_mint = create_mint(&env, &forwarder.program).await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&forwarder.program);
    let escrow_other = derive_associated_token_address(&escrow_authority, &other_mint);
    mint_to(&env, &other_mint, &escrow_other, AMOUNT).await?;
    let recipient_other = token_account(&env, &recipient, &other_mint).await?;
    let tx = forwarder
        .prove_unwrap(&env, wrapped, &owner, recipient)
        .await?;
    let unmoved = [escrow_other, recipient_other, forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        move |accounts| {
            accounts.segment[SOURCE].pubkey = escrow_other;
            accounts.segment[DESTINATION].pubkey = recipient_other;
        },
        "Error Code: WrongTokenAccountMint.",
        &unmoved,
    )
    .await
}

// The EVM forwarder reverts an unwrap to itself (BalanceMismatch: its balance
// does not grow by the amount). Released to the escrow authority, the tokens
// would never leave custody while the resource is spent.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_unwrap_whose_recipient_is_the_escrow_authority() -> anyhow::Result<()> {
    let (mut env, local, wrapped, owner) = wrapped().await?;
    let forwarder = &local.forwarder;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&forwarder.program);
    let tx = forwarder
        .prove_unwrap(&env, wrapped, &owner, escrow_authority)
        .await?;
    let unmoved = [forwarder.escrow_account()];
    refuses(
        &mut env,
        forwarder,
        tx,
        |_| {},
        "Error Code: UnwrapToEscrow.",
        &unmoved,
    )
    .await
}
