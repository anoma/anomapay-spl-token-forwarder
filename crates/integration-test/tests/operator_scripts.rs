//! The operator scripts against a local runtime: each command `ops.sh` runs
//! on a cluster does what it says there. Run with `ops.sh script-test`, which
//! installs the scripts' dependencies and builds the types they import.

use anoma_pa_solana_client::{
    derive_event_authority_pda, derive_pa_state_pda, derive_upgrade_authority_pda, pause_ix,
};
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anomapay_spl_token_forwarder_client::{
    CONFIG_VERSION, ConfigAccount, INSTRUCTIONS_SYSVAR_ID, decode_config,
    derive_associated_token_address, derive_forwarder_config_pda,
    derive_forwarder_escrow_authority, set_emergency_caller_ix, sha256,
};
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::scripts::{hex, run_script};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, Build, FORWARDER_SO, LocalForwarder, balance, give_sol, mint_to, new_mint,
    upgrade_authority,
};
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

/// The variables `forwarder init` reads: the transfer logic, `committee`
/// and `owner`.
fn init_vars(logic: [u8; 32], committee: &Pubkey, owner: &Pubkey) -> Vec<(&'static str, String)> {
    vec![
        ("STF_LOGIC_REF", hex(&logic)),
        ("STF_EMERGENCY_COMMITTEE", committee.to_string()),
        ("STF_OWNER", owner.to_string()),
    ]
}

/// A deploy's initialization: the config as requested, the upgrade authority
/// handed to the program, and, with STF_TOKEN_MINT, the mint's escrow.
#[tokio::test(flavor = "multi_thread")]
async fn init_initializes_a_deployed_forwarder_as_requested() -> anyhow::Result<()> {
    let (env, program) = setup::deployed(Build::Production).await?;
    let (committee, owner) = (Keypair::new().pubkey(), Keypair::new().pubkey());
    let mint = new_mint(&env).await?;
    let mut vars = init_vars(logic_ref().into(), &committee, &owner);
    vars.push(("STF_TOKEN_MINT", mint.to_string()));
    let ran = run_script(
        &env,
        &env.protocol_adapter.payer,
        "forwarder.ts",
        &["init"],
        &vars,
    )?;
    anyhow::ensure!(
        ran.success && ran.stdout.contains("✅ Config initialized"),
        "{ran}"
    );

    let stored = config(&env, &program).await?;
    let expected = ConfigAccount {
        protocol_adapter: env.protocol_adapter.program.to_bytes(),
        logic_ref: logic_ref().into(),
        emergency_committee: committee.to_bytes(),
        emergency_caller: [0; 32],
        version: CONFIG_VERSION,
        owner: owner.to_bytes(),
    };
    anyhow::ensure!(
        stored == expected,
        "the config is {stored:?}, not {expected:?}"
    );
    let authority = upgrade_authority(&env, &program).await?;
    let (pda, _) = derive_upgrade_authority_pda(&program);
    anyhow::ensure!(
        authority == Some(pda),
        "the upgrade authority is {authority:?}, not the program's PDA {pda}"
    );
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let escrow = derive_associated_token_address(&escrow_authority, &mint);
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
    let vars = init_vars(
        logic_ref().into(),
        &local.committee.pubkey(),
        &local.owner.pubkey(),
    );
    let ran = run_script(
        &env,
        &env.protocol_adapter.payer,
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
    let vars = init_vars(
        Keypair::new().pubkey().to_bytes(),
        &local.committee.pubkey(),
        &local.owner.pubkey(),
    );
    let ran = run_script(
        &env,
        &env.protocol_adapter.payer,
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

/// The development build's `dev_set_config_version`: the owner puts the
/// config at `version`, as an earlier build would have left it.
fn dev_set_config_version_ix(program: &Pubkey, owner: &Pubkey, version: u64) -> Instruction {
    let mut data =
        anoma_pa_solana_client::anchor_instruction_disc("dev_set_config_version").to_vec();
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

/// After an upgrade to a build that raises CONFIG_VERSION, the owner rotates
/// the logic ref.
#[tokio::test(flavor = "multi_thread")]
async fn reinitialize_rotates_the_logic_ref() -> anyhow::Result<()> {
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
    // The owner signs and pays as the script's wallet.
    give_sol(&env, &owner.pubkey(), 1_000_000_000).await?;
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

/// With the adapter paused and the caller named, the emergency caller moves
/// escrowed tokens to a recipient.
#[tokio::test(flavor = "multi_thread")]
async fn emergency_withdraw_moves_escrowed_tokens_to_the_recipient() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let LocalForwarder {
        forwarder,
        committee,
        ..
    } = &local;
    let payer = env.protocol_adapter.payer.pubkey();
    let (pa_state, _) = derive_pa_state_pda(&env.protocol_adapter.program);
    let caller = Keypair::new();
    env.send(&[pause_ix(&env.protocol_adapter.program, &payer)], &[])
        .await?;
    env.send(
        &[set_emergency_caller_ix(
            &forwarder.program,
            &committee.pubkey(),
            &pa_state,
            &caller.pubkey(),
        )],
        &[committee],
    )
    .await?;
    let escrow = forwarder.escrow_account();
    mint_to(&env, &forwarder.mint, &escrow, 1_000).await?;
    // The caller signs and pays, for the recipient's token account too.
    give_sol(&env, &caller.pubkey(), 1_000_000_000).await?;

    let recipient = Keypair::new().pubkey();
    let ran = run_script(
        &env,
        &caller,
        "forwarder.ts",
        &["emergency-withdraw"],
        &[
            ("STF_TOKEN_MINT", forwarder.mint.to_string()),
            ("STF_RECIPIENT", recipient.to_string()),
            ("STF_AMOUNT", "400".to_string()),
        ],
    )?;
    anyhow::ensure!(
        ran.success && ran.stdout.contains("✅ Withdrew 400"),
        "{ran}"
    );
    let recipient_account = derive_associated_token_address(&recipient, &forwarder.mint);
    anyhow::ensure!(
        balance(&env, &escrow).await? == 600 && balance(&env, &recipient_account).await? == 400,
        "the withdrawal did not move 400 from escrow to the recipient"
    );
    Ok(())
}

/// The forwarder's fixed accounts, and the listed mints' escrow accounts,
/// join the deployment's settlement lookup table.
#[tokio::test(flavor = "multi_thread")]
async fn lookup_table_adds_the_forwarders_accounts_to_the_deployments_table() -> anyhow::Result<()>
{
    let (env, program) = setup::deployed(Build::Production).await?;
    let mint = new_mint(&env).await?;
    let table = env.protocol_adapter.lookup_table.key;
    let ran = run_script(
        &env,
        &env.protocol_adapter.payer,
        "lookup-table.ts",
        &[],
        &[
            ("PA_LOOKUP_TABLE", table.to_string()),
            ("STF_TOKEN_MINTS", mint.to_string()),
        ],
    )?;
    anyhow::ensure!(ran.success, "{ran}");

    let data = env.protocol_adapter.rpc.get_account_data(&table).await?;
    let stored =
        solana_address_lookup_table_interface::state::AddressLookupTable::deserialize(&data)?
            .addresses
            .to_vec();
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    for key in [
        program,
        derive_forwarder_config_pda(&program).0,
        INSTRUCTIONS_SYSVAR_ID,
        derive_event_authority_pda(&program).0,
        escrow_authority,
        spl_token_interface::id(),
        derive_associated_token_address(&escrow_authority, &mint),
    ] {
        anyhow::ensure!(stored.contains(&key), "the table lacks {key}: {ran}");
    }
    Ok(())
}

/// The upgrade authority publishes the production IDL as the program's
/// canonical IDL account, and the runtime serves it.
#[tokio::test(flavor = "multi_thread")]
async fn publish_idl_writes_the_canonical_idl_account() -> anyhow::Result<()> {
    let (env, _) = setup::deployed(Build::Production).await?;
    let ran = run_script(
        &env,
        &env.protocol_adapter.payer,
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

/// Before `initialize` the wallet holding the upgrade authority upgrades
/// through the loader; after it, the owner upgrades through the program,
/// which runs the buffer's code and announces its hash.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_takes_the_path_the_upgrade_authority_leaves_and_installs_the_owners_buffer()
-> anyhow::Result<()> {
    let (env, _) = setup::deployed(Build::Production).await?;
    let payer = &env.protocol_adapter.payer;
    let ran = run_script(&env, payer, "upgrade-program.ts", &["path"], &[])?;
    anyhow::ensure!(ran.success && ran.stdout.trim() == "loader", "{ran}");

    let (env, local) = setup::local().await?;
    let owner = &local.owner;
    let ran = run_script(&env, owner, "upgrade-program.ts", &["path"], &[])?;
    anyhow::ensure!(ran.success && ran.stdout.trim() == "program", "{ran}");

    give_sol(&env, &owner.pubkey(), 1_000_000_000).await?;
    let buffer = env.write_buffer(FORWARDER_SO, owner.pubkey()).await?;
    let ran = run_script(
        &env,
        owner,
        "upgrade-program.ts",
        &["upgrade", &buffer.to_string()],
        &[],
    )?;
    let end = FORWARDER_SO
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |i| i + 1);
    let expected = hex(&sha256(&FORWARDER_SO[..end]));
    anyhow::ensure!(
        ran.success && ran.stdout.contains(&format!("upgraded to {expected}")),
        "{ran}"
    );
    Ok(())
}
