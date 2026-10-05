//! The forwarder's owner, as the EVM V2 forwarder's (OwnableUpgradeable): it
//! is stored in the config, rotates the logic ref, moves with
//! `transfer_ownership`, renounces for good, and alone upgrades the program,
//! whose upgrade authority is the program's own PDA.

use std::time::Duration;

use anoma_pa_solana_client::derive_program_data_address;
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anomapay_spl_token_forwarder_client::{
    ForwarderEvent, derive_forwarder_config_pda, reinitialize_ix, renounce_ownership_ix,
    transfer_ownership_ix, upgrade_ixs,
};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, FORWARDER_SO, LocalForwarder, config, events, executable_hash,
};
use solana_keypair::Keypair;
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use solana_signature::Signature;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// The owner-only call these tests make: reinitialize, which a config at
/// this build's version refuses with InvalidInitialization after the owner
/// check, so it changes nothing.
async fn reinitialize_as(
    env: &LocalEnv,
    local: &LocalForwarder,
    signer: &Keypair,
) -> anyhow::Result<Signature> {
    env.send(
        &[reinitialize_ix(
            &local.forwarder.program,
            &signer.pubkey(),
            Keypair::new().pubkey().to_bytes(),
        )],
        &[signer],
    )
    .await
}

const NOT_THE_OWNER: &str =
    "AnchorError caused by account: authority. Error Code: OwnableUnauthorizedAccount.";

#[tokio::test(flavor = "multi_thread")]
async fn transfer_ownership_refuses_a_signer_that_is_not_the_owner() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let stranger = Keypair::new();
    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        env.send(
            &[transfer_ownership_ix(
                &local.forwarder.program,
                &stranger.pubkey(),
                &stranger.pubkey(),
            )],
            &[&stranger],
        )
        .await,
    )
}

// OwnableUpgradeable.transferOwnership: OwnableInvalidOwner(address(0)).
#[tokio::test(flavor = "multi_thread")]
async fn transfer_ownership_refuses_the_zero_key() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    expect_integration_panic(Needle::Static("Error Code: OwnableInvalidOwner."))(
        env.send(
            &[transfer_ownership_ix(
                &local.forwarder.program,
                &local.owner.pubkey(),
                &Pubkey::default(),
            )],
            &[&local.owner],
        )
        .await,
    )
}

// OwnableUpgradeable.transferOwnership: the ownership moves at once,
// announced with OwnershipTransferred(previous, new).
#[tokio::test(flavor = "multi_thread")]
async fn transfer_ownership_moves_the_ownership_at_once_and_announces_it() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let (owner, successor) = (&local.owner, Keypair::new());
    let signature = env
        .send(
            &[transfer_ownership_ix(
                &local.forwarder.program,
                &owner.pubkey(),
                &successor.pubkey(),
            )],
            &[owner],
        )
        .await?;
    let events = events(
        &Executed::read(&env.protocol_adapter.rpc, &signature).await?,
        &local.forwarder.program,
    )?;
    let [ForwarderEvent::OwnershipTransferred(transferred)] = &events[..] else {
        anyhow::bail!("transfer_ownership emits {events:?}, not one OwnershipTransferred");
    };
    anyhow::ensure!(
        transferred.previous_owner == owner.pubkey().to_bytes()
            && transferred.new_owner == successor.pubkey().to_bytes(),
        "OwnershipTransferred {transferred:?} is not from {} to {}",
        owner.pubkey(),
        successor.pubkey()
    );
    let config = config(&env, &local.forwarder.program).await?;
    anyhow::ensure!(
        config.owner == successor.pubkey().to_bytes(),
        "the config's owner is {}, not the successor",
        Pubkey::new_from_array(config.owner)
    );

    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        reinitialize_as(&env, &local, owner).await,
    )?;
    expect_integration_panic(Needle::Static("Error Code: InvalidInitialization."))(
        reinitialize_as(&env, &local, &successor).await,
    )
}

// Mirrors OwnableUpgradeable.renounceOwnership on the EVM forwarder: the
// owner becomes the zero address, announced, and neither the logic-ref
// rotation nor an upgrade can run again.
#[tokio::test(flavor = "multi_thread")]
async fn renounced_ownership_is_announced_and_closes_reinitialize_and_upgrade() -> anyhow::Result<()>
{
    let (env, local) = setup::local().await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    let signature = env
        .send(
            &[renounce_ownership_ix(&program, &owner.pubkey())],
            &[owner],
        )
        .await?;
    let events = events(
        &Executed::read(&env.protocol_adapter.rpc, &signature).await?,
        &local.forwarder.program,
    )?;
    let [ForwarderEvent::OwnershipTransferred(transferred)] = &events[..] else {
        anyhow::bail!("renounce_ownership emits {events:?}, not one OwnershipTransferred");
    };
    anyhow::ensure!(
        transferred.previous_owner == owner.pubkey().to_bytes() && transferred.new_owner == [0; 32],
        "OwnershipTransferred {transferred:?} is not from the owner to the zero key"
    );

    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        reinitialize_as(&env, &local, owner).await,
    )?;
    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        env.send(
            &upgrade_ixs(
                &program,
                &owner.pubkey(),
                &Keypair::new().pubkey(),
                &owner.pubkey(),
            ),
            &[owner],
        )
        .await,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_refuses_a_signer_that_is_not_the_owner() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let stranger = Keypair::new();
    expect_integration_panic(Needle::Static(NOT_THE_OWNER))(
        env.send(
            &upgrade_ixs(
                &local.forwarder.program,
                &stranger.pubkey(),
                &Keypair::new().pubkey(),
                &stranger.pubkey(),
            ),
            &[&stranger],
        )
        .await,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_refuses_an_account_that_is_not_a_loader_buffer() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let program = local.forwarder.program;
    let owner = &local.owner;
    expect_integration_panic(Needle::Static("Error Code: InvalidUpgradeBuffer."))(
        env.send(
            &upgrade_ixs(
                &program,
                &owner.pubkey(),
                &derive_forwarder_config_pda(&program).0,
                &owner.pubkey(),
            ),
            &[owner],
        )
        .await,
    )
}

// The program hands the owner's buffer to its PDA, so a buffer someone other
// than the owner wrote is refused by the loader.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_refuses_a_buffer_someone_other_than_the_owner_wrote() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let buffer = env
        .write_buffer(FORWARDER_SO, env.protocol_adapter.payer.pubkey())
        .await?;
    let owner = &local.owner;
    expect_integration_panic(Needle::Static("Incorrect authority provided"))(
        env.send(
            &upgrade_ixs(
                &local.forwarder.program,
                &owner.pubkey(),
                &buffer,
                &owner.pubkey(),
            ),
            &[owner],
        )
        .await,
    )
}

// UUPS upgradeToAndCall: the owner replaces the code, announced with
// ERC1967's Upgraded, naming the code by its executable hash. The test
// upgrades to the build it already runs.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_replaces_the_code_with_the_owners_buffer_announces_its_executable_hash_and_runs_it_from_the_next_slot()
-> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    let rpc = &env.protocol_adapter.rpc;
    let expected = executable_hash(FORWARDER_SO);
    let buffer = env.write_buffer(FORWARDER_SO, owner.pubkey()).await?;
    let buffer_rent = rpc.get_balance(&buffer).await?;
    let spill = Keypair::new().pubkey();

    let signature = env
        .send(
            &upgrade_ixs(&program, &owner.pubkey(), &buffer, &spill),
            &[owner],
        )
        .await?;

    let events = events(
        &Executed::read(&env.protocol_adapter.rpc, &signature).await?,
        &local.forwarder.program,
    )?;
    let [ForwarderEvent::Upgraded(upgraded)] = &events[..] else {
        anyhow::bail!("upgrade emits {events:?}, not one Upgraded");
    };
    anyhow::ensure!(
        upgraded.executable_hash == expected,
        "Upgraded announces {:02x?}, not the buffer's executable hash {expected:02x?}",
        upgraded.executable_hash
    );
    let program_data = rpc
        .get_account_data(&derive_program_data_address(&program))
        .await?;
    let code = &program_data[UpgradeableLoaderState::size_of_programdata_metadata()..];
    anyhow::ensure!(
        executable_hash(code) == expected,
        "the program does not run the buffer's code"
    );
    anyhow::ensure!(
        rpc.get_account_with_commitment(&buffer, rpc.commitment())
            .await?
            .value
            .is_none(),
        "the loader closes the buffer"
    );
    anyhow::ensure!(
        rpc.get_balance(&spill).await? == buffer_rent,
        "the buffer's rent goes to spill"
    );

    // The loader makes upgraded code visible from the slot after the
    // upgrade's.
    let landed = rpc.get_signature_statuses(&[signature]).await?.value[0]
        .as_ref()
        .map(|status| status.slot)
        .ok_or_else(|| anyhow::anyhow!("the runtime has no status of the upgrade {signature}"))?;
    while rpc.get_slot().await? <= landed {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    expect_integration_panic(Needle::Static("Error Code: InvalidInitialization."))(
        reinitialize_as(&env, &local, owner).await,
    )
}
