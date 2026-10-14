//! A kind table that lists the forwarder's mint is installable: a wrap proven
//! against it is refused while the deployment holds any other root, and
//! settles once the adapter's owner installs its root.

use anoma_pa_testkit::environment::{ProtocolAdapter as _, Refusal};
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_rm_risc0::compliance::KindTableEntry;
use anoma_rm_risc0::kind_table::KindTable;
use anoma_rm_risc0::resource::{Resource, kind};
use anomapay_spl_token_forwarder_integration_test::fixtures::{self, ShieldedOwner, WrapTerms};
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup;
use anyhow::Context;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use transfer_witness::calculate_label_ref;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

#[tokio::test(flavor = "multi_thread")]
async fn settles_a_wrap_proven_against_a_kind_table_once_the_owner_installs_its_root()
-> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    let forwarder = local.forwarder;
    // The table lists the transfer resource under the forwarder's label for
    // its mint, so a wrap of that mint is of a kind the table lists and is
    // proven against the table; a transaction none of whose kinds a table
    // lists is proven against the empty table, which every adapter accepts.
    let listed = Resource {
        logic_ref: logic_ref(),
        label_ref: calculate_label_ref(&forwarder.program.to_bytes(), &forwarder.mint.to_bytes()),
        ..Resource::default()
    };
    let kind_point = kind(&listed)
        .context("the wrapped resource has no kind")?
        .to_affine()
        .to_encoded_point(false)
        .as_bytes()
        .to_vec();
    let kind_table = KindTable::from_entries(vec![KindTableEntry {
        logic_ref: listed.logic_ref,
        label_ref: listed.label_ref,
        kind_point,
    }])
    .context("the table is not a kind table")?;
    let root = kind_table.root();
    let held = env.protocol_adapter.state().await?.kind_table_root;
    anyhow::ensure!(
        held != <[u8; 32]>::from(root),
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
        tx.aggregation_instance()?.kind_table_root == root,
        "the wrap must be proven against the table"
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

    env.protocol_adapter.set_kind_table_root(root).await?;
    execute_tx(&mut env, tx).await?;
    anyhow::ensure!(
        balances(&env, &accounts).await? == [before[0] - AMOUNT, before[1] + AMOUNT],
        "the wrap moves {AMOUNT} from the user's account to the escrow's"
    );
    Ok(())
}
