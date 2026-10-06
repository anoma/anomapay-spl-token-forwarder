//! What a test checks of a settlement the forwarder refuses: the refusal it
//! logs, and that no tokens moved.

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_solana_integration_test::forwarders::CallAccounts;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::execute_tx;
use anoma_pa_testkit::transaction::Transaction;
use surfpool_sdk::Pubkey;

use crate::setup::{Forwarder, balance};

/// The token balances of `accounts`.
pub async fn balances<P>(env: &Environment<P>, accounts: &[Pubkey]) -> anyhow::Result<Vec<u64>> {
    let mut amounts = Vec::with_capacity(accounts.len());
    for account in accounts {
        amounts.push(balance(env, account).await?);
    }
    Ok(amounts)
}

/// Settling `tx` with what `rewrite` makes of the submitter's accounts fails
/// with `error`, the log line naming the refusal, and the token accounts
/// `unmoved` hold what they held before.
pub async fn refuses<P>(
    env: &mut Environment<P>,
    forwarder: &Forwarder,
    tx: Transaction,
    rewrite: impl Fn(&mut CallAccounts) + Send + Sync + 'static,
    error: &'static str,
    unmoved: &[Pubkey],
) -> anyhow::Result<()>
where
    P: Prover,
{
    let before = balances(env, unmoved).await?;
    forwarder.rewrite(env, rewrite);
    let settled = execute_tx(env, tx).await;
    forwarder.restore(env);
    expect_integration_panic(Needle::Static(error))(settled)?;
    let after = balances(env, unmoved).await?;
    anyhow::ensure!(
        after == before,
        "tokens moved: {unmoved:?} held {before:?}, now {after:?}"
    );
    Ok(())
}
