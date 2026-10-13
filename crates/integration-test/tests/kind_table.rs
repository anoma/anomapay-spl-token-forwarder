//! The kind table anoma/risc0-kind-tables generates for solana-devnet is
//! installable: a wrap proven against it is refused while the deployment
//! holds any other root, and settles once the adapter's owner installs
//! the root risc0-kind-tables publishes for devnet.

use anoma_pa_testkit::environment::{ProtocolAdapter as _, Refusal};
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_risc0_kind_tables::SolanaCluster;
use anoma_risc0_kind_tables::{table, tokens};
use anoma_rm_risc0::compliance::KindTableEntry;
use anoma_rm_risc0::kind_table::KindTable;
use anomapay_spl_token_forwarder_integration_test::fixtures::{self, ShieldedOwner, WrapTerms};
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup;
use anyhow::Context;
use surfpool_sdk::Pubkey;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

#[tokio::test(flavor = "multi_thread")]
async fn settles_a_wrap_proven_against_the_solana_devnet_kind_table_once_the_owner_installs_its_root()
-> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    // The table lists devnet's mint under the forwarder's label, so a wrap of
    // it is of a kind the table lists and is proven against the table; a
    // transaction none of whose kinds a table lists is proven against the
    // empty table, which every adapter accepts.
    let listed = tokens::spl_on(SolanaCluster::Devnet)
        .first()
        .context("risc0-kind-tables lists no solana-devnet mint")?;
    let forwarder = setup::serve_mint_at(
        &mut env,
        local.forwarder.program,
        Pubkey::new_from_array(*listed.mint.as_bytes()),
    )
    .await?;
    let kind_table = KindTable::from_entries(
        table::staging::table(SolanaCluster::Devnet)?
            .entries
            .iter()
            .map(KindTableEntry::from)
            .collect(),
    )
    .map_err(|error| anyhow::anyhow!("the devnet table is not a kind table: {error:?}"))?;
    let published = table::staging::root(SolanaCluster::Devnet)?;
    anyhow::ensure!(
        kind_table.root() == published,
        "the devnet table's entries do not have the root published for devnet"
    );
    let held = env.protocol_adapter.state().await?.kind_table_root;
    anyhow::ensure!(
        held != <[u8; 32]>::from(published),
        "the deployment must hold another table for the wrap to be refused first"
    );

    let owner = ShieldedOwner::seeded("kind-table/owner");
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
        &kind_table,
        "kind-table/wrap",
    )?;
    let tx = prove_actions(&env, &[wrap.witnesses]).await?;
    anyhow::ensure!(
        tx.aggregation_instance()?.kind_table_root == published,
        "the wrap must be proven against the devnet table"
    );

    let accounts = [forwarder.user_account(), forwarder.escrow_account()];
    let before = balances(&env, &accounts).await?;
    // The same transaction settles below. A kind table neither stored nor
    // empty is refused before the proof is verified.
    let refusal = env.protocol_adapter.submit(tx.clone()).await?.err();
    anyhow::ensure!(
        refusal == Some(Refusal::UnacceptedKindTableRoot),
        "the adapter returned {refusal:?}, not a refusal of the kind table"
    );
    anyhow::ensure!(
        balances(&env, &accounts).await? == before,
        "a refused wrap moved tokens"
    );

    env.protocol_adapter.set_kind_table_root(published).await?;
    execute_tx(&mut env, tx).await?;
    anyhow::ensure!(
        balances(&env, &accounts).await? == [before[0] - AMOUNT, before[1] + AMOUNT],
        "the wrap moves {AMOUNT} from the user's account to the escrow's"
    );
    Ok(())
}
