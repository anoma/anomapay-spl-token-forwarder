//! The forwarder's config: who may rotate its logic ref, and the guards a
//! direct caller hits. Everything forward_call does past its caller check
//! needs the adapter as the CPI caller, so those behaviours are tested
//! through settlement (wrap.rs, unwrap.rs).

use anoma_pa_solana_client::{
    anchor_instruction_disc, derive_event_authority_pda, derive_pa_state_pda,
};
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anomapay_spl_token_forwarder_client::{
    CONFIG_VERSION, ConfigAccount, INSTRUCTIONS_SYSVAR_ID, decode_config,
    derive_forwarder_config_pda, encode_unwrap_forwarder_input, reinitialize_ix,
    set_emergency_caller_ix,
};
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::setup::{self, Build, LocalForwarder};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

async fn config(env: &LocalEnv, program: &Pubkey) -> anyhow::Result<ConfigAccount> {
    let (config, _) = derive_forwarder_config_pda(program);
    Ok(decode_config(
        &env.protocol_adapter.rpc.get_account_data(&config).await?,
    )?)
}

/// A logic ref no resource has: 32 random bytes.
fn random_ref() -> [u8; 32] {
    Keypair::new().pubkey().to_bytes()
}

/// The development build's `dev_set_config_version`: the owner puts the
/// config at `version`, as an earlier build would have left it.
fn dev_set_config_version_ix(program: &Pubkey, owner: &Pubkey, version: u64) -> Instruction {
    let mut data = anchor_instruction_disc("dev_set_config_version").to_vec();
    data.extend_from_slice(&version.to_le_bytes());
    Instruction {
        program_id: *program,
        accounts: vec![
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(derive_forwarder_config_pda(program).0, false),
        ],
        data,
    }
}

/// Reinitializing as `signer` fails with `error` and leaves the config as it
/// was.
async fn refuses_reinitialize(
    env: &LocalEnv,
    local: &LocalForwarder,
    signer: &Keypair,
    error: &'static str,
) -> anyhow::Result<()> {
    let program = local.forwarder.program;
    let before = config(env, &program).await?;
    expect_integration_panic(Needle::Static(error))(
        env.send(
            &[reinitialize_ix(&program, &signer.pubkey(), random_ref())],
            &[signer],
        )
        .await,
    )?;
    let after = config(env, &program).await?;
    anyhow::ensure!(
        after == before,
        "the config moved from {before:?} to {after:?}"
    );
    Ok(())
}

// Only the owner rotates, as only the EVM forwarder's owner calls
// upgradeToAndCall with the reinitializer.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_a_signer_that_is_not_the_owner() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    refuses_reinitialize(
        &env,
        &local,
        &Keypair::new(),
        "AnchorError caused by account: authority. Error Code: OwnableUnauthorizedAccount.",
    )
    .await
}

// Mirrors OpenZeppelin's reinitializer(n): InvalidInitialization once the
// version is n. A config this build initialized is at its version, so
// rotating it takes a build that raises CONFIG_VERSION.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_a_config_already_at_this_builds_version() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    refuses_reinitialize(
        &env,
        &local,
        &local.owner,
        "Error Code: InvalidInitialization.",
    )
    .await
}

// A config an earlier build initialized sits below this build's
// CONFIG_VERSION; reinitialize then rotates the ref and records the version,
// once.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_rotates_the_logic_ref_once_for_a_config_below_this_builds_version()
-> anyhow::Result<()> {
    let (env, local) = setup::local_with(Build::Development).await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    env.send(
        &[dev_set_config_version_ix(
            &program,
            &owner.pubkey(),
            CONFIG_VERSION - 1,
        )],
        &[owner],
    )
    .await?;

    let rotated = random_ref();
    env.send(
        &[reinitialize_ix(&program, &owner.pubkey(), rotated)],
        &[owner],
    )
    .await?;
    let after = config(&env, &program).await?;
    anyhow::ensure!(
        after.logic_ref == rotated && after.version == CONFIG_VERSION,
        "the config is {after:?}, not rotated to {rotated:02x?} at version {CONFIG_VERSION}"
    );
    refuses_reinitialize(&env, &local, owner, "Error Code: InvalidInitialization.").await
}

// Mirrors ForwarderBase.t.sol: test_forwardCall_reverts_if_the_pa_is_not_the_caller.
// The forwarder reads the current top-level instruction's program id from the
// instructions sysvar; a direct call sees itself, not the adapter.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_forward_call_that_is_not_a_cpi_from_the_adapter() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let program = local.forwarder.program;
    let input = encode_unwrap_forwarder_input(
        &Keypair::new().pubkey().to_bytes(),
        1000,
        &Keypair::new().pubkey().to_bytes(),
    );
    let mut data = anchor_instruction_disc("forward_call").to_vec();
    data.extend_from_slice(&<[u8; 32]>::from(logic_ref()));
    data.extend_from_slice(&(input.len() as u32).to_le_bytes());
    data.extend_from_slice(&input);
    let forward_call = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new_readonly(derive_forwarder_config_pda(&program).0, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR_ID, false),
            AccountMeta::new_readonly(derive_event_authority_pda(&program).0, false),
            AccountMeta::new_readonly(program, false),
        ],
        data,
    };
    expect_integration_panic(Needle::Static("Error Code: UnauthorizedCaller."))(
        env.send(&[forward_call], &[]).await,
    )
}

// Mirrors EmergencyMigratableForwarderBase.t.sol: test_setEmergencyCaller_reverts_if_the_caller_is_not_the_emergency_committee
#[tokio::test(flavor = "multi_thread")]
async fn refuses_set_emergency_caller_from_a_non_committee_signer() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let impostor = Keypair::new();
    let (pa_state, _) = derive_pa_state_pda(&env.protocol_adapter.program);
    expect_integration_panic(Needle::Static("Error Code: UnauthorizedCaller."))(
        env.send(
            &[set_emergency_caller_ix(
                &local.forwarder.program,
                &impostor.pubkey(),
                &pa_state,
                &Keypair::new().pubkey(),
            )],
            &[&impostor],
        )
        .await,
    )
}
