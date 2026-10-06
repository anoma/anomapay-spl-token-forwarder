//! A wrap and an unwrap through the forwarder, settled by the adapter.

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::execute_tx;
use anomapay_spl_token_forwarder_integration_test::fixtures::ShieldedOwner;
use anomapay_spl_token_forwarder_integration_test::setup::{self, Forwarder, balance};
use solana_keypair::Keypair;
use solana_signer::Signer;
use spl_associated_token_account_interface::address::get_associated_token_address;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

/// The user wraps `AMOUNT` into a resource `owner` holds, then the owner
/// unwraps it to a fresh recipient: the user's tokens move into escrow, then
/// out to the recipient.
async fn wraps_then_unwraps<P>(
    env: &mut Environment<P>,
    forwarder: &Forwarder,
) -> anyhow::Result<()>
where
    P: Prover,
{
    let owner = ShieldedOwner::seeded("wrap-then-unwrap/owner");
    let (user, escrow) = (forwarder.user_account(), forwarder.escrow_account());
    let user_before = balance(env, &user).await?;
    let escrow_before = balance(env, &escrow).await?;

    let wrapped = forwarder
        .wrap(env, &owner, AMOUNT, 1, "wrap-then-unwrap/wrap")
        .await?;
    anyhow::ensure!(balance(env, &user).await? == user_before - AMOUNT);
    anyhow::ensure!(balance(env, &escrow).await? == escrow_before + AMOUNT);

    let recipient = Keypair::new().pubkey();
    let tx = forwarder
        .prove_unwrap(env, wrapped, &owner, recipient)
        .await?;
    execute_tx(env, tx).await?;
    let recipient_account = get_associated_token_address(&recipient, &forwarder.mint);
    anyhow::ensure!(balance(env, &recipient_account).await? == AMOUNT);
    anyhow::ensure!(balance(env, &escrow).await? == escrow_before);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn local_wraps_then_unwraps() -> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    wraps_then_unwraps(&mut env, &local.forwarder).await
}

#[cfg(feature = "e2e")]
#[tokio::test(flavor = "multi_thread")]
async fn e2e_test_wraps_then_unwraps() -> anyhow::Result<()> {
    let (mut env, forwarder) = setup::e2e().await?;
    wraps_then_unwraps(&mut env, &forwarder).await
}
