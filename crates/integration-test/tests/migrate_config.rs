//! The in-place upgrade from the previous release, whose config held an
//! emergency committee and caller: until the owner's `migrate_config`, the
//! upgraded forwarder serves wraps and unwraps and refuses every owner-only
//! instruction; the migration keeps the config's values in this release's
//! layout, returns the freed rent to the owner, and runs once.

use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anoma_pa_testkit::execute_tx;
use anomapay_spl_token_forwarder_client::{
    CONFIG_VERSION, ConfigAccount, derive_forwarder_config_pda, migrate_config_ix,
    transfer_ownership_ix,
};
use anomapay_spl_token_forwarder_integration_test::fixtures::ShieldedOwner;
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::setup::{self, LocalForwarder, balance, config};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// 100 tokens at the mint's 6 decimals.
const AMOUNT: u64 = 100_000_000;

/// The config's size in this release's layout: the discriminator, the
/// adapter, the logic ref, the version and the owner.
const CONFIG_SIZE: usize = 8 + 32 + 32 + 8 + 32;

const NOT_THE_OWNER: &str = "Error Code: OwnableUnauthorizedAccount.";

/// The config account's (size, lamports).
async fn config_account(env: &LocalEnv, program: &Pubkey) -> anyhow::Result<(usize, u64)> {
    let account = env
        .protocol_adapter
        .rpc
        .get_account(&derive_forwarder_config_pda(program).0)
        .await?;
    Ok((account.data.len(), account.lamports))
}

/// The user wraps `AMOUNT` and the shielded owner unwraps it to a fresh
/// recipient, both settled through the adapter.
async fn wrap_then_unwrap(
    env: &mut LocalEnv,
    local: &LocalForwarder,
    seed: &str,
) -> anyhow::Result<()> {
    let forwarder = &local.forwarder;
    let owner = ShieldedOwner::seeded(&format!("{seed}/owner"));
    let wrapped = forwarder
        .wrap(env, &owner, AMOUNT, 1, &format!("{seed}/wrap"))
        .await?;
    let recipient = Keypair::new().pubkey();
    let tx = forwarder
        .prove_unwrap(env, wrapped, &owner, recipient)
        .await?;
    execute_tx(env, tx).await?;
    let received = balance(
        env,
        &spl_associated_token_account_interface::address::get_associated_token_address(
            &recipient,
            &forwarder.mint,
        ),
    )
    .await?;
    anyhow::ensure!(
        received == AMOUNT,
        "the recipient holds {received}, not {AMOUNT}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn before_the_migration_the_upgraded_forwarder_serves_and_refuses_its_owner()
-> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_from_the_previous_release().await?;
    wrap_then_unwrap(&mut env, &local, "before-migration").await?;
    // The owner the previous layout records is read from bytes no key holds.
    let (program, owner) = (local.forwarder.program, &local.owner);
    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        env.send(
            &[transfer_ownership_ix(
                &program,
                &owner.pubkey(),
                &Keypair::new().pubkey(),
            )],
            &[owner],
        )
        .await,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migration_keeps_the_configs_values_and_returns_the_freed_rent_to_the_owner()
-> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_from_the_previous_release().await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    let (_, lamports_before) = config_account(&env, &program).await?;
    let owner_before = env
        .protocol_adapter
        .rpc
        .get_balance(&owner.pubkey())
        .await?;

    env.send(&[migrate_config_ix(&program, &owner.pubkey())], &[owner])
        .await?;

    let stored = config(&env, &program).await?;
    let expected = ConfigAccount {
        protocol_adapter: env.protocol_adapter.program.to_bytes(),
        logic_ref: logic_ref().into(),
        version: CONFIG_VERSION,
        owner: owner.pubkey().to_bytes(),
    };
    anyhow::ensure!(
        stored == expected,
        "the config is {stored:?}, not {expected:?}"
    );
    let (size, lamports) = config_account(&env, &program).await?;
    let rent = env
        .protocol_adapter
        .rpc
        .get_minimum_balance_for_rent_exemption(CONFIG_SIZE)
        .await?;
    anyhow::ensure!(
        size == CONFIG_SIZE && lamports == rent,
        "the config holds {size} bytes and {lamports} lamports, not {CONFIG_SIZE} and its rent {rent}"
    );
    let owner_after = env
        .protocol_adapter
        .rpc
        .get_balance(&owner.pubkey())
        .await?;
    anyhow::ensure!(
        owner_after - owner_before == lamports_before - rent,
        "the owner gained {}, not the freed {}",
        owner_after - owner_before,
        lamports_before - rent
    );

    // The owner's instructions and the settlements both run on the new layout.
    let new_owner = Keypair::new();
    env.send(
        &[transfer_ownership_ix(
            &program,
            &owner.pubkey(),
            &new_owner.pubkey(),
        )],
        &[owner],
    )
    .await?;
    anyhow::ensure!(config(&env, &program).await?.owner == new_owner.pubkey().to_bytes());
    wrap_then_unwrap(&mut env, &local, "after-migration").await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migration_refuses_anyone_but_the_owner() -> anyhow::Result<()> {
    let (env, local) = setup::upgraded_from_the_previous_release().await?;
    let stranger = Keypair::new();
    setup::give_sol(&env, &stranger.pubkey(), 1_000_000_000)?;
    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        env.send(
            &[migrate_config_ix(
                &local.forwarder.program,
                &stranger.pubkey(),
            )],
            &[&stranger],
        )
        .await,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migration_runs_once() -> anyhow::Result<()> {
    let (env, local) = setup::upgraded_from_the_previous_release().await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    env.send(&[migrate_config_ix(&program, &owner.pubkey())], &[owner])
        .await?;
    expect_integration_panic(Needle::Static("Error Code: NotPreviousLayout."))(
        env.send(&[migrate_config_ix(&program, &owner.pubkey())], &[owner])
            .await,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migration_refuses_a_config_this_release_initialized() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let owner = &local.owner;
    setup::give_sol(&env, &owner.pubkey(), 1_000_000_000)?;
    expect_integration_panic(Needle::Static("Error Code: NotPreviousLayout."))(
        env.send(
            &[migrate_config_ix(&local.forwarder.program, &owner.pubkey())],
            &[owner],
        )
        .await,
    )
}
