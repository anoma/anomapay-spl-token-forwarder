//! The forwarder's config: who may rotate its logic ref, and the guards a
//! direct caller hits. Everything forward_call does past its caller check
//! needs the adapter as the CPI caller, so those behaviours are tested
//! through settlement (wrap.rs, unwrap.rs).

use anchor_lang::{InstructionData, ToAccountMetas};
use anoma_pa_solana_client::derive_event_authority_pda;
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::client::{accounts, args};
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::constants::CONFIG_VERSION;
use anomapay_spl_token_forwarder_client::{
    derive_forwarder_config_pda, encode_unwrap_forwarder_input, reinitialize_ix,
};
use anomapay_spl_token_forwarder_integration_test::logic::{logic_ref, random_logic_ref};
use anomapay_spl_token_forwarder_integration_test::setup::{self, OwnedForwarder, config, same};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Reinitializing to `logic_ref` as `signer` fails with `error` and leaves
/// the config as it was.
async fn refuses_reinitialize(
    env: &LocalEnv,
    local: &OwnedForwarder,
    signer: &Keypair,
    logic_ref: [u8; 32],
    error: &'static str,
) -> anyhow::Result<()> {
    let program = local.forwarder.program;
    let before = config(env, &program).await?;
    expect_integration_panic(Needle::Static(error))(
        env.send(
            &[reinitialize_ix(&program, &signer.pubkey(), logic_ref)],
            &[signer],
        )
        .await,
    )?;
    let after = config(env, &program).await?;
    anyhow::ensure!(
        same(&after, &before),
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
        random_logic_ref().into(),
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
        random_logic_ref().into(),
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
    let (env, local) = setup::local_below_this_builds_version().await?;
    let (program, owner) = (local.forwarder.program, &local.owner);

    let rotated = random_logic_ref().into();
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
    refuses_reinitialize(
        &env,
        &local,
        owner,
        random_logic_ref().into(),
        "Error Code: InvalidInitialization.",
    )
    .await
}

// Mirrors ForwarderBaseUpgradeable's ZeroLogicRefNotAllowed, which its
// initializer, and so a reinitializer, raises for the zero logic ref.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_refuses_the_zero_logic_ref() -> anyhow::Result<()> {
    let (env, local) = setup::local_below_this_builds_version().await?;
    refuses_reinitialize(
        &env,
        &local,
        &local.owner,
        [0; 32],
        "Error Code: ZeroLogicRefNotAllowed.",
    )
    .await
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
    let forward_call = Instruction {
        program_id: program,
        accounts: accounts::ForwardCall {
            config: derive_forwarder_config_pda(&program).0,
            ix_sysvar: solana_sdk_ids::sysvar::instructions::ID,
            event_authority: derive_event_authority_pda(&program).0,
            program,
        }
        .to_account_metas(None),
        data: args::ForwardCall {
            logic_ref: logic_ref().into(),
            input,
        }
        .data(),
    };
    expect_integration_panic(Needle::Static("Error Code: ProtocolAdapterMismatch."))(
        env.send(&[forward_call], &[]).await,
    )
}
