//! The adapter runs the forwarder calls of an action in the order of its
//! resources, so a wrap can pull the tokens an unwrap before it in the list
//! releases, as the ERC20 forwarder's `forwarder_call_order` checks.

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, latest_root, prove_actions};
use anoma_rm_risc0::resource::Resource;
use anomapay_spl_token_forwarder_integration_test::fixtures::{self, ShieldedOwner, WrapTerms};
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup::{self, Forwarder, balance};
use anyhow::Context;
use solana_signer::Signer;
use transfer_library::TransferLogic;
use transfer_library::action::Layout;

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
    let root_before = latest_root(env).await?;
    execute_tx(env, unwrap_first)
        .await
        .context("failed to settle the action with the unwrap first")?;
    let root_after = latest_root(env).await?;
    println!("the unwrap first in the list: the latest root moves {root_before} -> {root_after}");
    anyhow::ensure!(
        root_after != root_before,
        "the action with the unwrap first adds its commitments"
    );
    let after = balances(env, &accounts).await?;
    println!("the unwrap first in the list: user and escrow hold {after:?}");
    anyhow::ensure!(
        after == [0, escrow_before + amount],
        "the balances end where the first wrap left them: the wrap pulls back the {amount} the unwrap released"
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
    let unwrap = fixtures::unwrap_flow::<TransferLogic>(
        forwarder.program,
        forwarder.mint,
        wrapped,
        owner,
        forwarder.user.pubkey(),
        path,
    )?;
    let wrap = fixtures::wrap_flow(
        forwarder.program,
        forwarder.mint,
        owner,
        amount,
        "call-order/wrap",
    );
    let terms = WrapTerms {
        amount,
        nonce: 2,
        ed25519_ix_index: 0,
    };
    // The unwrap's consumed resource, the wrap's consumed one and the wrap's
    // created one, with the unwrap's created one at `unwrapped_at`.
    let unwrapped_at = if unwrap_first { 1 } else { 2 };
    let [spent, unwrapped] = unwrap.resources();
    let [pulled, deposited] = wrap.deposit.resources();
    let mut resources = vec![spent, pulled, deposited];
    resources.insert(unwrapped_at, unwrapped);
    let deposited_at = resources.len() - 1;
    let layout = Layout::new(resources).context("failed to lay out the action")?;
    let root = layout.root();

    let [spend, burn] = unwrap.logic_witnesses(
        layout.resource(unwrapped_at),
        root,
        fixtures::unwrap_auth::<TransferLogic>(owner, root),
    );
    let (auth, _) = fixtures::wrap_auth(&wrap, &forwarder.user, terms, root)?;
    let seed = format!("call-order/unwrap-first={unwrap_first}");
    let [pull, deposit] = wrap.logic_witnesses(
        layout.resource(deposited_at),
        root,
        auth,
        &fixtures::discovery_pk(),
        &mut fixtures::rng("call-order/wrap"),
    );
    let mut logic_witnesses = vec![spend, pull, deposit];
    logic_witnesses.insert(unwrapped_at, burn);
    let action = layout
        .action(
            logic_witnesses,
            fixtures::compliance(&seed, anoma_pa_testkit::fixtures::kind_tree()),
        )
        .context("failed to build the action's witnesses")?;
    prove_actions(env, &[fixtures::witnesses(action)]).await
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
