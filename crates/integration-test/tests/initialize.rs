//! Initializing the forwarder: who may, the zero values it refuses, and what
//! it stores and announces.

use anoma_pa_solana_client::{derive_program_data_address, derive_upgrade_authority_pda};
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::constants::CONFIG_VERSION;
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::events::{
    Initialized, OwnershipTransferred,
};
use anomapay_spl_token_forwarder_client::{derive_forwarder_config_pda, initialize_ix};
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, Build, config, discriminator, event_order, expected_config, give_sol, same,
    upgrade_authority,
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// The forwarder `program`'s initialize by its upgrade authority, the
/// adapter's payer, for the adapter and the transfer logic, with `owner`.
fn initialize(env: &LocalEnv, program: &Pubkey, owner: &Pubkey) -> Instruction {
    initialize_ix(
        program,
        &env.protocol_adapter.payer_pubkey(),
        &env.protocol_adapter.program,
        logic_ref().into(),
        owner,
    )
}

/// Initializing with `ix` fails with `error` and creates no config.
async fn refuses(
    env: &LocalEnv,
    program: &Pubkey,
    ix: Instruction,
    signers: &[&Keypair],
    error: &'static str,
) -> anyhow::Result<()> {
    expect_integration_panic(Needle::Static(error))(env.send(&[ix], signers).await)?;
    let (config, _) = derive_forwarder_config_pda(program);
    anyhow::ensure!(
        env.protocol_adapter
            .rpc
            .get_account_with_commitment(&config, env.protocol_adapter.rpc.commitment())
            .await?
            .value
            .is_none(),
        "a refused initialize created the config {config}"
    );
    Ok(())
}

// The program's upgrade authority, the deployer, initializes it: the EVM
// proxy runs its initializer atomically at deployment, so no one else ever
// can.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_initialize_signed_by_anyone_but_the_programs_upgrade_authority()
-> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let intruder = Keypair::new();
    // The signer pays for the config, so the intruder holds enough for it.
    give_sol(&env, &intruder.pubkey(), 1_000_000_000)?;
    let ix = initialize_ix(
        &program,
        &intruder.pubkey(),
        &env.protocol_adapter.program,
        logic_ref().into(),
        &intruder.pubkey(),
    );
    refuses(
        &env,
        &program,
        ix,
        &[&intruder],
        "AnchorError caused by account: program_data. Error Code: Unauthorized.",
    )
    .await
}

// Another program with the same upgrade authority is the cheapest forgery of
// the upgrade-authority check.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_the_upgrade_authority_of_another_programs_program_data() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let other = env.deploy_test_forwarder()?;
    let mut ix = initialize(&env, &program, &Keypair::new().pubkey());
    let program_data = derive_program_data_address(&program);
    let slot = ix
        .accounts
        .iter()
        .position(|meta| meta.pubkey == program_data)
        .expect("initialize passes the program data");
    ix.accounts[slot].pubkey = derive_program_data_address(&other);
    refuses(
        &env,
        &program,
        ix,
        &[],
        "AnchorError caused by account: program_data. Error Code: Unauthorized.",
    )
    .await
}

// Mirrors OwnableUpgradeable's initializer: OwnableInvalidOwner(address(0)).
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_zero_owner() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let ix = initialize(&env, &program, &Pubkey::default());
    refuses(&env, &program, ix, &[], "Error Code: OwnableInvalidOwner.").await
}

// Mirrors ForwarderBase.t.sol:
// test_constructor_reverts_if_the_{protocol_adapter_address,logic_ref}_is_zero

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_zero_protocol_adapter() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let ix = initialize_ix(
        &program,
        &env.protocol_adapter.payer_pubkey(),
        &Pubkey::default(),
        logic_ref().into(),
        &Keypair::new().pubkey(),
    );
    refuses(
        &env,
        &program,
        ix,
        &[],
        "Error Code: ZeroProtocolAdapterNotAllowed.",
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_zero_logic_ref() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let ix = initialize_ix(
        &program,
        &env.protocol_adapter.payer_pubkey(),
        &env.protocol_adapter.program,
        [0; 32],
        &Keypair::new().pubkey(),
    );
    refuses(
        &env,
        &program,
        ix,
        &[],
        "Error Code: ZeroLogicRefNotAllowed.",
    )
    .await
}

// Mirrors ForwarderBase.t.sol getProtocolAdapter/getLogicRef.
// Mirrors OpenZeppelin's initializer: the owner is set and announced first
// (OwnershipTransferred from the zero address), then the config records the
// version it was initialized at, this build's, and announces it. The upgrade
// authority moves to the program, as a UUPS implementation authorizes its
// own upgrades.
#[tokio::test(flavor = "multi_thread")]
async fn stores_its_configuration_announces_it_and_hands_the_upgrade_authority_to_the_program()
-> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let owner = Keypair::new().pubkey();
    let signature = env.send(&[initialize(&env, &program, &owner)], &[]).await?;

    let config = config(&env, &program).await?;
    let expected = expected_config(&env, &owner);
    anyhow::ensure!(
        same(&config, &expected),
        "the config is {config:?}, not {expected:?}"
    );

    let (upgrade_pda, _) = derive_upgrade_authority_pda(&program);
    let authority = upgrade_authority(&env, &program).await?;
    anyhow::ensure!(
        authority == Some(upgrade_pda),
        "the program's upgrade authority is {authority:?}, not its PDA {upgrade_pda}"
    );

    let executed = Executed::read(&env.protocol_adapter.rpc, &signature).await?;
    let order = event_order(&executed, &program)?;
    anyhow::ensure!(
        order
            == [
                discriminator::<OwnershipTransferred>(),
                discriminator::<Initialized>()
            ],
        "initialize emits the events {order:02x?}, not OwnershipTransferred then Initialized"
    );
    let transferred = executed.events::<OwnershipTransferred>(&program)?.remove(0);
    let initialized = executed.events::<Initialized>(&program)?.remove(0);
    anyhow::ensure!(
        transferred.previous_owner == Pubkey::default() && transferred.new_owner == owner,
        "OwnershipTransferred {transferred:?} is not from the zero key to {owner}"
    );
    anyhow::ensure!(
        initialized.version == CONFIG_VERSION,
        "Initialized announces version {}, not {CONFIG_VERSION}",
        initialized.version
    );
    Ok(())
}
