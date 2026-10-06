//! The kind table anoma/risc0-kind-tables generates for solana-devnet is
//! installable: a wrap proven against it is refused under the commitment the
//! deployment holds, and settles once the adapter's owner installs the
//! commitment risc0-kind-tables publishes for devnet.

use anoma_pa_solana_client::set_kind_table_commitment_ix;
use anoma_pa_testkit::environment::Refusal;
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_risc0_kind_tables::SolanaCluster;
use anoma_risc0_kind_tables::table;
use anoma_rm_risc0::compliance::{KindTableEntry, hash_kind_table_entries};
use anomapay_spl_token_forwarder_integration_test::fixtures::{self, ShieldedOwner, WrapTerms};
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup;
use solana_signer::Signer;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

#[tokio::test(flavor = "multi_thread")]
async fn settles_a_wrap_proven_against_the_solana_devnet_kind_table_once_the_owner_installs_its_commitment()
-> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    let forwarder = &local.forwarder;
    let entries: Vec<KindTableEntry> = table::staging::table(SolanaCluster::Devnet)?
        .entries
        .iter()
        .map(KindTableEntry::from)
        .collect();
    let published = table::staging::commitment(SolanaCluster::Devnet)?;
    anyhow::ensure!(
        hash_kind_table_entries(&entries) == published,
        "the devnet table's entries do not hash to the commitment published for devnet"
    );
    let held = env.protocol_adapter.state().await?.kind_table_commitment;
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
        entries,
        "kind-table/wrap",
    )?;
    forwarder
        .submitter
        .authorize(forwarder.user.pubkey(), nonce, wrap.authorization);
    let tx = prove_actions(&env, &[wrap.witnesses]).await?;

    let accounts = [forwarder.user_account(), forwarder.escrow_account()];
    let before = balances(&env, &accounts).await?;
    // The same transaction settles below. A proof against another kind table
    // is an invalid aggregation proof.
    let refusal = env.protocol_adapter.submit(tx.clone()).await?.err();
    anyhow::ensure!(
        refusal == Some(Refusal::InvalidAggregationProof),
        "the adapter returned {refusal:?}, not a refusal of the proof"
    );
    anyhow::ensure!(
        balances(&env, &accounts).await? == before,
        "a refused wrap moved tokens"
    );

    env.send(
        &[set_kind_table_commitment_ix(
            &env.protocol_adapter.program,
            &env.protocol_adapter.payer.pubkey(),
            published.into(),
        )],
        &[],
    )
    .await?;
    execute_tx(&mut env, tx).await?;
    anyhow::ensure!(
        balances(&env, &accounts).await? == [before[0] - AMOUNT, before[1] + AMOUNT],
        "the wrap moves {AMOUNT} from the user's account to the escrow's"
    );
    Ok(())
}
