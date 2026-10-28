//! The adapter runs the forwarder calls of an action in the order of its
//! resources, so a wrap can pull the tokens an unwrap before it in the list
//! releases, as the ERC20 forwarder's `forwarder_call_order` checks.

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_rm_risc0::resource::Resource;
use anomapay_spl_token_forwarder_integration_test::fixtures::{
    self, Parts, ShieldedOwner, WrapTerms,
};
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup::{self, Forwarder, balance};
use anyhow::Context;
use solana_signer::Signer;

/// The user wraps its whole balance first, so the wrap of the action can only
/// pull the tokens the unwrap of the same action releases to the user.
async fn runs_the_forwarder_calls_in_the_order_of_the_resources<P>(
    env: &mut Environment<P>,
    forwarder: &Forwarder,
) -> anyhow::Result<()>
where
    P: Prover,
{
    let owner = ShieldedOwner::seeded("call-order/owner");
    let (user, escrow) = (forwarder.user_account(), forwarder.escrow_account());
    let amount = balance(env, &user).await?;
    let escrow_before = balance(env, &escrow).await?;
    let wrapped = forwarder
        .wrap(env, &owner, amount, 1, "call-order/first-wrap")
        .await?;
    anyhow::ensure!(
        balance(env, &user).await? == 0,
        "the user holds no tokens after the first wrap"
    );
    // The first wrap used up the user's approval; the action's wrap needs
    // its own, as a user gives before each wrap.
    setup::approve(env, &forwarder.program, &user, &forwarder.user, amount).await?;

    let wrap_first = prove_unwrap_and_wrap(env, forwarder, &owner, wrapped, amount, false).await?;
    let accounts = [user, escrow];
    let before = balances(env, &accounts).await?;
    let Err(error) = execute_tx(env, wrap_first).await else {
        anyhow::bail!("the wrap must not pull tokens before the unwrap releases them");
    };
    let logs = format!("{error:?}");
    println!("the wrap first in the list: {logs}");
    // The SPL Token program refuses the transfer from the user's empty
    // account, and its failure aborts the forwarder's CPI, which fails with
    // the token program's error (0x1, InsufficientFunds).
    for line in [
        "Program log: Error: insufficient funds".to_string(),
        format!(
            "Program {} failed: custom program error: 0x1",
            forwarder.program
        ),
    ] {
        anyhow::ensure!(
            logs.contains(&line),
            "with the wrap first in the list, the settlement must fail with {line:?}: {logs}"
        );
    }
    anyhow::ensure!(
        balances(env, &accounts).await? == before,
        "the refused action moved tokens"
    );

    let unwrap_first = prove_unwrap_and_wrap(env, forwarder, &owner, wrapped, amount, true).await?;
    execute_tx(env, unwrap_first)
        .await
        .context("failed to settle the action with the unwrap first")?;
    let after = balances(env, &accounts).await?;
    println!("the unwrap first in the list: user and escrow hold {after:?}");
    anyhow::ensure!(
        after == [0, escrow_before + amount],
        "the wrap pulls the {amount} the unwrap releases back into the escrow, which holds {}",
        escrow_before + amount
    );
    Ok(())
}

/// One action that unwraps `wrapped`, `owner`'s resource of `amount`, to the
/// user, and wraps `amount` of the user's tokens under the forwarder nonce 2.
/// The unwrap's created resource, which makes its call, comes before the
/// wrap's consumed resource, which makes the wrap's, exactly if
/// `unwrap_first`.
async fn prove_unwrap_and_wrap<P>(
    env: &Environment<P>,
    forwarder: &Forwarder,
    owner: &ShieldedOwner,
    wrapped: Resource,
    amount: u64,
    unwrap_first: bool,
) -> anyhow::Result<Transaction>
where
    P: Prover,
{
    let path = env
        .protocol_adapter
        .commitment_tree
        .path_to(wrapped.commitment())?;
    let unwrap = fixtures::unwrap_parts(
        forwarder.program,
        forwarder.mint,
        wrapped,
        owner,
        forwarder.user.pubkey(),
        path,
    )?;
    let wrap = fixtures::wrap_parts(
        forwarder.program,
        forwarder.mint,
        &forwarder.user,
        owner,
        WrapTerms {
            amount,
            nonce: 2,
            ed25519_ix_index: 0,
        },
        "call-order/wrap",
    )?;
    let (
        Parts {
            consumed: spent,
            created: unwrapped,
        },
        Parts {
            consumed: pulled,
            created: deposited,
        },
    ) = (unwrap, wrap);
    let parts = if unwrap_first {
        vec![spent, unwrapped, pulled, deposited]
    } else {
        vec![spent, pulled, unwrapped, deposited]
    };
    let seed = format!("call-order/unwrap-first={unwrap_first}");
    prove_actions(env, &[fixtures::action(parts, &seed)?]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn local_runs_the_forwarder_calls_in_the_order_of_the_resources() -> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    runs_the_forwarder_calls_in_the_order_of_the_resources(&mut env, &local.forwarder).await
}

#[cfg(feature = "e2e")]
#[tokio::test(flavor = "multi_thread")]
async fn e2e_test_runs_the_forwarder_calls_in_the_order_of_the_resources() -> anyhow::Result<()> {
    let (mut env, forwarder) = setup::e2e().await?;
    runs_the_forwarder_calls_in_the_order_of_the_resources(&mut env, &forwarder).await
}
