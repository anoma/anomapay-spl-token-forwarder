//! Wraps and unwraps through the forwarder, settled by the adapter, on the
//! local environment and the e2e environment.

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::execute_tx;
use anomapay_spl_token_forwarder_integration_test::fixtures::ShieldedOwner;
use anomapay_spl_token_forwarder_integration_test::setup::{self, Forwarder, balance};

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

/// The user wraps `AMOUNT` and twice `AMOUNT` in one transaction, under two
/// nonces: each wrap names its own signature check, and both move the user's
/// tokens into escrow.
async fn two_wraps_settle_in_one_transaction<P>(
    env: &mut Environment<P>,
    forwarder: &Forwarder,
) -> anyhow::Result<()>
where
    P: Prover,
{
    let owner = ShieldedOwner::seeded("two-wraps/owner");
    let (user, escrow) = (forwarder.user_account(), forwarder.escrow_account());
    let user_before = balance(env, &user).await?;
    let escrow_before = balance(env, &escrow).await?;

    let tx = forwarder
        .prove_wraps(
            env,
            &owner,
            &[(AMOUNT, 1), (2 * AMOUNT, 2)],
            "two-wraps/wrap",
        )
        .await?;
    execute_tx(env, tx).await?;
    anyhow::ensure!(balance(env, &user).await? == user_before - 3 * AMOUNT);
    anyhow::ensure!(balance(env, &escrow).await? == escrow_before + 3 * AMOUNT);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn local_wraps_then_unwraps() -> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    local
        .forwarder
        .wraps_then_unwraps(&mut env, AMOUNT, "wrap-then-unwrap")
        .await
}

#[cfg(feature = "e2e")]
#[tokio::test(flavor = "multi_thread")]
async fn e2e_test_wraps_then_unwraps() -> anyhow::Result<()> {
    let (mut env, forwarder) = setup::e2e().await?;
    forwarder
        .wraps_then_unwraps(&mut env, AMOUNT, "wrap-then-unwrap")
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn local_two_wraps_settle_in_one_transaction() -> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    two_wraps_settle_in_one_transaction(&mut env, &local.forwarder).await
}

#[cfg(feature = "e2e")]
#[tokio::test(flavor = "multi_thread")]
async fn e2e_test_two_wraps_settle_in_one_transaction() -> anyhow::Result<()> {
    let (mut env, forwarder) = setup::e2e().await?;
    two_wraps_settle_in_one_transaction(&mut env, &forwarder).await
}
