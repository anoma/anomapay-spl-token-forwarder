//! The operator scripts against a local runtime: each command `ops.sh` runs
//! on a cluster does what it says there. Run with `ops.sh script-test`, which
//! installs the scripts' dependencies and builds the types they import.

use anoma_pa_solana_client::derive_upgrade_authority_pda;
use anomapay_spl_token_forwarder_client::derive_forwarder_escrow_authority;
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::constants::CONFIG_VERSION;
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::scripts::{hex, run_script};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, Build, FORWARDER_SO, balance, config, executable_hash, expected_config, give_sol,
    new_mint, same, upgrade_authority,
};
use solana_keypair::Keypair;
use solana_signer::Signer;
use spl_associated_token_account_interface::address::get_associated_token_address;
use surfpool_sdk::Pubkey;

/// The variables `forwarder init` reads: the transfer logic and `owner`.
fn init_vars(logic: [u8; 32], owner: &Pubkey) -> Vec<(&'static str, String)> {
    vec![
        ("STF_LOGIC_REF", hex(&logic)),
        ("STF_OWNER", owner.to_string()),
    ]
}

/// A deploy's initialization: the config as requested, the upgrade authority
/// handed to the program, and, with STF_TOKEN_MINT, the mint's escrow.
#[tokio::test(flavor = "multi_thread")]
async fn init_initializes_a_deployed_forwarder_as_requested() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let owner = Keypair::new().pubkey();
    let mint = new_mint(&env).await?;
    let mut vars = init_vars(logic_ref().into(), &owner);
    vars.push(("STF_TOKEN_MINT", mint.to_string()));
    let ran = run_script(
        &env,
        env.protocol_adapter.payer()?,
        "forwarder.ts",
        &["init"],
        &vars,
    )?;
    anyhow::ensure!(
        ran.success && ran.stdout.contains("✅ Config initialized"),
        "{ran}"
    );

    let stored = config(&env, &program).await?;
    let expected = expected_config(&env, &owner);
    anyhow::ensure!(
        same(&stored, &expected),
        "the config is {stored:?}, not {expected:?}"
    );
    let authority = upgrade_authority(&env, &program).await?;
    let (pda, _) = derive_upgrade_authority_pda(&program);
    anyhow::ensure!(
        authority == Some(pda),
        "the upgrade authority is {authority:?}, not the program's PDA {pda}"
    );
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let escrow = get_associated_token_address(&escrow_authority, &mint);
    anyhow::ensure!(
        balance(&env, &escrow).await? == 0,
        "the mint's escrow account {escrow} is not created"
    );
    Ok(())
}

// The operator's init is idempotent only for the config it would create.

#[tokio::test(flavor = "multi_thread")]
async fn init_accepts_an_existing_config_that_holds_the_requested_values() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let vars = init_vars(logic_ref().into(), &local.owner.pubkey());
    let ran = run_script(
        &env,
        env.protocol_adapter.payer()?,
        "forwarder.ts",
        &["init"],
        &vars,
    )?;
    anyhow::ensure!(
        ran.success
            && ran
                .stdout
                .contains("already initialized with the requested values"),
        "{ran}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn init_refuses_an_existing_config_that_differs_from_the_request() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let vars = init_vars(Keypair::new().pubkey().to_bytes(), &local.owner.pubkey());
    let ran = run_script(
        &env,
        env.protocol_adapter.payer()?,
        "forwarder.ts",
        &["init"],
        &vars,
    )?;
    anyhow::ensure!(
        !ran.success
            && ran
                .stderr
                .contains("already exists with a different logic ref"),
        "{ran}"
    );
    Ok(())
}

/// After an upgrade to a build that raises CONFIG_VERSION, the owner rotates
/// the logic ref.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_rotates_the_logic_ref() -> anyhow::Result<()> {
    let (env, local) = setup::local_below_this_builds_version().await?;
    let (program, owner) = (local.forwarder.program, &local.owner);
    // The owner signs and pays as the script's wallet.
    give_sol(&env, &owner.pubkey(), 1_000_000_000)?;
    let rotated = Keypair::new().pubkey().to_bytes();
    let ran = run_script(
        &env,
        owner,
        "forwarder.ts",
        &["reinitialize"],
        &[("STF_LOGIC_REF", hex(&rotated))],
    )?;
    anyhow::ensure!(
        ran.success && ran.stdout.contains("✅ Logic ref rotated"),
        "{ran}"
    );
    let stored = config(&env, &program).await?;
    anyhow::ensure!(
        stored.logic_ref == rotated && stored.version == CONFIG_VERSION,
        "the config is {stored:?}, not rotated to {rotated:02x?} at version {CONFIG_VERSION}"
    );
    Ok(())
}

/// The upgrade authority publishes the production IDL as the program's
/// canonical IDL account, and the runtime serves it.
#[tokio::test(flavor = "multi_thread")]
async fn publish_idl_writes_the_canonical_idl_account() -> anyhow::Result<()> {
    let (env, _) = setup::deployed(Build::Production).await?;
    let ran = run_script(
        &env,
        env.protocol_adapter.payer()?,
        "publish-idl.ts",
        &["target/idl/spl_token_forwarder.json"],
        &[],
    )?;
    anyhow::ensure!(
        ran.success
            && ran
                .stdout
                .contains("written as the upgrade authority; the cluster serves it"),
        "{ran}"
    );
    Ok(())
}

/// Once `initialize` has handed the upgrade authority to the program, no
/// wallet writes the canonical IDL account, the owner included.
#[tokio::test(flavor = "multi_thread")]
async fn publish_idl_refuses_the_owner_once_initialized() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let (pda, _) = derive_upgrade_authority_pda(&local.forwarder.program);
    let ran = run_script(
        &env,
        &local.owner,
        "publish-idl.ts",
        &["target/idl/spl_token_forwarder.json"],
        &[],
    )?;
    anyhow::ensure!(
        !ran.success
            && ran.stderr.contains(&format!(
                "cannot write {}'s canonical IDL account",
                local.forwarder.program
            ))
            && ran
                .stderr
                .contains(&format!("the program's upgrade authority is {pda}")),
        "{ran}"
    );
    Ok(())
}

/// Before `initialize` the wallet holding the upgrade authority upgrades
/// through the loader; after it, the owner upgrades through the program,
/// which runs the buffer's code and announces its hash.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_takes_the_path_the_upgrade_authority_leaves_and_installs_the_owners_buffer()
-> anyhow::Result<()> {
    let (env, _) = setup::deployed(Build::Production).await?;
    let payer = env.protocol_adapter.payer()?;
    let ran = run_script(&env, payer, "upgrade-program.ts", &["path"], &[])?;
    anyhow::ensure!(ran.success && ran.stdout.trim() == "loader", "{ran}");

    let (env, local) = setup::local().await?;
    let owner = &local.owner;
    let ran = run_script(&env, owner, "upgrade-program.ts", &["path"], &[])?;
    anyhow::ensure!(ran.success && ran.stdout.trim() == "program", "{ran}");

    give_sol(&env, &owner.pubkey(), 1_000_000_000)?;
    let buffer = env.write_buffer(FORWARDER_SO, owner.pubkey()).await?;
    let ran = run_script(
        &env,
        owner,
        "upgrade-program.ts",
        &["upgrade", &buffer.to_string()],
        &[],
    )?;
    let expected = hex(&executable_hash(FORWARDER_SO));
    anyhow::ensure!(
        ran.success && ran.stdout.contains(&format!("upgraded to {expected}")),
        "{ran}"
    );
    Ok(())
}
