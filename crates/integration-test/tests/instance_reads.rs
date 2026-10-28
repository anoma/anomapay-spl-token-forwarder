//! The client's forwarder instance (`anomapay_spl_token_forwarder_client::
//! instance`), the EVM bindings of the ERC20 forwarder on Solana, against a
//! deployed forwarder: each read answers what the test set up, and a
//! forwarder never initialized has no config to answer from. The
//! emergency-migrating build's reads are checked in `emergency_migrating.rs`,
//! after its incident.

use anomapay_spl_token_forwarder_client::derive_forwarder_config_pda;
use anomapay_spl_token_forwarder_client::instance::{InstanceError, SplTokenForwarder};
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::setup::{self, Build};
use solana_keypair::Keypair;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// The release the production build's IDL records, which its `version`
/// answers.
fn idl_version() -> anyhow::Result<String> {
    let idl: serde_json::Value =
        serde_json::from_str(include_str!("../../client/idls/spl_token_forwarder.json"))?;
    idl["metadata"]["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("the IDL records no version: {}", idl["metadata"]))
}

// The forwarder is initialized by the adapter's payer for the adapter and the
// transfer resource's logic ref with a new owner, who hands the ownership to
// another key through the instance's builder. Every read of the instance
// answers that.
#[tokio::test(flavor = "multi_thread")]
async fn every_read_answers_what_the_forwarder_was_set_up_with() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let rpc = &env.protocol_adapter.rpc;
    let program = local.forwarder.program;
    let instance = SplTokenForwarder::at(rpc, program);
    anyhow::ensure!(
        instance.program_id() == program,
        "the instance is not at {program}"
    );

    let new_owner = Keypair::new().pubkey();
    env.send(
        &[instance.transfer_ownership_ix(&local.owner.pubkey(), &new_owner)],
        &[&local.owner],
    )
    .await?;

    let read_logic_ref = instance.get_logic_ref().await?;
    let expected_logic_ref: [u8; 32] = logic_ref().into();
    println!("get_logic_ref: {read_logic_ref:02x?}");
    anyhow::ensure!(
        read_logic_ref == expected_logic_ref,
        "get_logic_ref() is {read_logic_ref:02x?}, the forwarder was initialized with the \
         transfer resource's {expected_logic_ref:02x?}"
    );

    let adapter = instance.get_protocol_adapter().await?;
    println!("get_protocol_adapter: {adapter}");
    anyhow::ensure!(
        adapter == env.protocol_adapter.program,
        "get_protocol_adapter() is {adapter}, the forwarder was initialized for {}",
        env.protocol_adapter.program
    );

    let owner = instance.owner().await?;
    println!("owner: {owner}");
    anyhow::ensure!(
        owner == new_owner,
        "owner() is {owner}, the ownership was transferred to {new_owner}"
    );

    let payer = env.protocol_adapter.payer_pubkey();
    let version = instance.version(&payer).await?;
    let expected_version = idl_version()?;
    println!("version: {version}");
    anyhow::ensure!(
        version == expected_version,
        "version() is {version}, the IDL records the release {expected_version}"
    );
    Ok(())
}

/// Requires `result`, the read `read` of a forwarder never initialized, to
/// be the error that its config `config` is missing.
fn missing<T: std::fmt::Debug>(
    config: Pubkey,
    read: &str,
    result: Result<T, InstanceError>,
) -> anyhow::Result<()> {
    println!("{read}: {result:?}");
    anyhow::ensure!(
        matches!(&result, Err(InstanceError::AccountMissing { address }) if *address == config),
        "{read} on a forwarder never initialized is {result:?}, not AccountMissing of its \
         config {config}"
    );
    Ok(())
}

// A forwarder deployed but never initialized has no config: every read of the
// config says so, naming the account, rather than decoding nothing; its
// `version` needs no account and answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_forwarder_never_initialized_has_no_config_to_read() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let instance = SplTokenForwarder::at(&env.protocol_adapter.rpc, program);
    let config = derive_forwarder_config_pda(&program).0;

    missing(config, "get_logic_ref", instance.get_logic_ref().await)?;
    missing(
        config,
        "get_protocol_adapter",
        instance.get_protocol_adapter().await,
    )?;
    missing(config, "owner", instance.owner().await)?;

    let version = instance
        .version(&env.protocol_adapter.payer_pubkey())
        .await?;
    println!("version: {version}");
    anyhow::ensure!(
        version == idl_version()?,
        "version() is {version} on a forwarder never initialized, the IDL records {}",
        idl_version()?
    );
    Ok(())
}
