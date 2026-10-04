//! A wrap and an unwrap through the forwarder, settled by the adapter.

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_testkit::environment::{CommitmentTree, Environment as _, ProtocolAdapter, Prover};
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, prove_actions};
use anomapay_spl_token_forwarder_client::derive_forwarder_escrow_authority;
use anomapay_spl_token_forwarder_integration_test::fixtures::{self, ShieldedOwner, WrapTerms};
use anomapay_spl_token_forwarder_integration_test::setup::{self, Forwarder, balance};
use solana_keypair::Keypair;
use solana_signer::Signer;

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
    P: Prover<Transaction = Transaction>,
{
    let owner = ShieldedOwner::seeded("wrap-then-unwrap/owner");
    let (escrow, _) = derive_forwarder_escrow_authority(&forwarder.program);
    let user = forwarder.user.pubkey();
    let user_before = balance(env, &user, &forwarder.mint).await?;
    let escrow_before = balance(env, &escrow, &forwarder.mint).await?;

    let nonce = 1;
    let wrap = fixtures::wrap(
        forwarder.program,
        forwarder.mint,
        &forwarder.user,
        &owner,
        WrapTerms {
            amount: AMOUNT,
            nonce,
            ed25519_ix_index: 0,
        },
        "wrap-then-unwrap/wrap",
    )?;
    forwarder
        .submitter
        .authorize(user, nonce, wrap.authorization.clone());
    let tx = prove_actions(env, &[wrap.witnesses]).await?;
    execute_tx(env, tx).await?;
    anyhow::ensure!(balance(env, &user, &forwarder.mint).await? == user_before - AMOUNT);
    anyhow::ensure!(balance(env, &escrow, &forwarder.mint).await? == escrow_before + AMOUNT);

    let recipient = Keypair::new().pubkey();
    let path = env
        .protocol_adapter()
        .commitment_tree()
        .path_to(wrap.created.commitment())?;
    let unwrap = fixtures::unwrap(
        forwarder.program,
        forwarder.mint,
        wrap.created,
        &owner,
        recipient,
        path,
    )?;
    let tx = prove_actions(env, &[unwrap]).await?;
    execute_tx(env, tx).await?;
    anyhow::ensure!(balance(env, &recipient, &forwarder.mint).await? == AMOUNT);
    anyhow::ensure!(balance(env, &escrow, &forwarder.mint).await? == escrow_before);
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
