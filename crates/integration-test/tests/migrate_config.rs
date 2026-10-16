//! The in-place upgrade from the previous release, whose config held an
//! emergency committee and caller: until the owner's `migrate_config`, the
//! upgraded forwarder serves wraps and unwraps and every owner-only
//! instruction refuses the config; the migration keeps the config's values
//! in this release's layout, returns the freed rent to the owner, and runs
//! once.

use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anomapay_spl_token_forwarder_client::{
    CONFIG_ACCOUNT_SIZE, decode_config, derive_forwarder_config_pda, migrate_config_ix,
    transfer_ownership_ix,
};
use anomapay_spl_token_forwarder_integration_test::setup::{self, config, expected_config};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// The config account's data and lamports.
async fn config_account(env: &LocalEnv, program: &Pubkey) -> anyhow::Result<(Vec<u8>, u64)> {
    let account = env
        .protocol_adapter
        .rpc
        .get_account(&derive_forwarder_config_pda(program).0)
        .await?;
    Ok((account.data, account.lamports))
}

#[tokio::test(flavor = "multi_thread")]
async fn before_the_migration_the_upgraded_forwarder_serves_and_refuses_its_owner()
-> anyhow::Result<()> {
    let (mut env, local) = setup::upgraded_from_the_previous_release().await?;
    local.forwarder.wraps_then_unwraps(&mut env).await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    expect_integration_panic(Needle::Static("Error Code: ConfigNotMigrated."))(
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
    let rpc = &env.protocol_adapter.rpc;
    let (_, lamports_before) = config_account(&env, &program).await?;
    let owner_before = rpc.get_balance(&owner.pubkey()).await?;

    env.send(&[migrate_config_ix(&program, &owner.pubkey())], &[owner])
        .await?;

    let (data, lamports) = config_account(&env, &program).await?;
    let stored = decode_config(&data)?;
    let expected = expected_config(&env, &owner.pubkey());
    anyhow::ensure!(
        stored == expected,
        "the config is {stored:?}, not {expected:?}"
    );
    let rent = rpc
        .get_minimum_balance_for_rent_exemption(CONFIG_ACCOUNT_SIZE)
        .await?;
    anyhow::ensure!(
        lamports == rent,
        "the config holds {lamports} lamports, not its rent {rent}"
    );
    let gained = rpc.get_balance(&owner.pubkey()).await? - owner_before;
    anyhow::ensure!(
        gained == lamports_before - rent,
        "the owner gained {gained}, not the freed {}",
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
    local.forwarder.wraps_then_unwraps(&mut env).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_migration_refuses_anyone_but_the_owner() -> anyhow::Result<()> {
    let (env, local) = setup::upgraded_from_the_previous_release().await?;
    let stranger = Keypair::new();
    setup::give_sol(&env, &stranger.pubkey(), 1_000_000_000)?;
    expect_integration_panic(Needle::Static("Error Code: OwnableUnauthorizedAccount."))(
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
