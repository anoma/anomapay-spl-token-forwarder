//! The forwarder on one of the adapter harness's environments, as the
//! ERC20 forwarder's tests add theirs to pa-evm's.

use std::sync::Arc;

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_solana_integration_test::state::actors::default_signer;
use anoma_pa_solana_integration_test::state::pa::pa_program;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::transaction::Transaction;
use anomapay_spl_token_forwarder_client::{
    INSTRUCTIONS_SYSVAR_ID, create_ata_idempotent_ix, derive_associated_token_address,
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, initialize_ix,
};
use anyhow::Context;
use solana_keypair::Keypair;
use solana_program_pack::Pack;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

use crate::submitter::SplTokenForwarder;

/// The forwarder build the local tests load: the deterministic build at the
/// local addresses (`dev.sh test-program`).
const FORWARDER_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder.so");
const LOCALNET: &str = include_str!("../../../env/localnet.env");
#[cfg(feature = "e2e")]
const DEVNET: &str = include_str!("../../../env/devnet.env");

/// The tokens a test's user starts with, at the mint's 6 decimals: 1000.
pub const USER_TOKENS: u64 = 1_000_000_000;

/// The forwarder's address in an env file.
fn forwarder_address(env_file: &str) -> anyhow::Result<Pubkey> {
    let value = env_file
        .lines()
        .find_map(|line| line.strip_prefix("FORWARDER_PROGRAM_ID="))
        .context("the env file names no FORWARDER_PROGRAM_ID")?;
    value
        .parse()
        .with_context(|| format!("FORWARDER_PROGRAM_ID={value} is not base58"))
}

/// The forwarder on an environment: the submitter registered with the
/// adapter, a mint it serves, and a user holding the mint's tokens who has
/// approved the forwarder's escrow authority as delegate.
pub struct Forwarder {
    pub submitter: Arc<SplTokenForwarder>,
    pub program: Pubkey,
    pub mint: Pubkey,
    pub user: Keypair,
}

/// The forwarder as the local environment deploys it: also the owner and
/// emergency committee it was initialized with.
pub struct LocalForwarder {
    pub forwarder: Forwarder,
    pub owner: Keypair,
    pub committee: Keypair,
}

/// The local environment with the forwarder deployed and initialized for the
/// adapter and the transfer resource's logic ref.
pub async fn local() -> anyhow::Result<(
    anoma_pa_solana_integration_test::envs::local::Environment,
    LocalForwarder,
)> {
    let mut env = anoma_pa_solana_integration_test::envs::local::Environment::setup_bare().await?;
    let program = forwarder_address(LOCALNET)?;
    let payer = default_signer(&env)?;
    env.deploy_program(program, FORWARDER_SO, payer.pubkey())?;
    let (owner, committee) = (Keypair::new(), Keypair::new());
    env.send(
        &[initialize_ix(
            &program,
            &payer.pubkey(),
            &pa_program(&env)?,
            crate::logic::logic_ref().into(),
            &committee.pubkey(),
            &owner.pubkey(),
        )],
        &[],
    )
    .await
    .context("failed to initialize the forwarder")?;
    let forwarder = serve(&mut env, program).await?;
    Ok((
        env,
        LocalForwarder {
            forwarder,
            owner,
            committee,
        },
    ))
}

/// A fork of devnet with the forwarder devnet runs, checked to serve the
/// devnet adapter and the transfer resource's logic ref.
#[cfg(feature = "e2e")]
pub async fn e2e() -> anyhow::Result<(
    anoma_pa_solana_integration_test::envs::e2e::Environment,
    Forwarder,
)> {
    let mut env = anoma_pa_solana_integration_test::envs::e2e::Environment::setup_bare().await?;
    use anomapay_spl_token_forwarder_client::decode_config;

    let program = forwarder_address(DEVNET)?;
    let (config_address, _) = derive_forwarder_config_pda(&program);
    let config = decode_config(
        &env.protocol_adapter
            .rpc
            .get_account_data(&config_address)
            .await
            .with_context(|| format!("the forwarder {program} has no config {config_address}"))?,
    )?;
    let pa = pa_program(&env)?;
    anyhow::ensure!(
        config.protocol_adapter == pa.to_bytes(),
        "the devnet forwarder serves the adapter {}, not {pa}",
        Pubkey::new_from_array(config.protocol_adapter)
    );
    anyhow::ensure!(
        config.logic_ref == <[u8; 32]>::from(crate::logic::logic_ref()),
        "the devnet forwarder serves the logic ref {}, the tests build {}",
        anoma_rm_risc0::Digest::from_bytes(config.logic_ref),
        crate::logic::logic_ref()
    );
    let forwarder = serve(&mut env, program).await?;
    Ok((env, forwarder))
}

/// Has the forwarder `program` serve a new mint on `env`: the mint, its
/// escrow, a user holding `USER_TOKENS` who has approved the escrow authority
/// to move them, the submitter registered with the adapter, and the
/// forwarder's fixed accounts in the settlement lookup table.
async fn serve<P>(env: &mut Environment<P>, program: Pubkey) -> anyhow::Result<Forwarder>
where
    P: Prover<Transaction = Transaction>,
{
    let payer = default_signer(env)?.pubkey();
    let rpc = env.protocol_adapter.rpc.clone();
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);

    let mint = Keypair::new();
    let space = spl_token_interface::state::Mint::LEN;
    let rent = rpc
        .get_minimum_balance_for_rent_exemption(space)
        .await
        .context("failed to read the mint's rent")?;
    let token_program = spl_token_interface::id();
    env.send(
        &[
            solana_system_interface::instruction::create_account(
                &payer,
                &mint.pubkey(),
                rent,
                space as u64,
                &token_program,
            ),
            spl_token_interface::instruction::initialize_mint2(
                &token_program,
                &mint.pubkey(),
                &payer,
                None,
                6,
            )?,
            create_ata_idempotent_ix(&payer, &escrow_authority, &mint.pubkey()),
        ],
        &[&mint],
    )
    .await
    .context("failed to create the mint and its escrow")?;

    let user = Keypair::new();
    let user_ata = derive_associated_token_address(&user.pubkey(), &mint.pubkey());
    env.send(
        &[
            create_ata_idempotent_ix(&payer, &user.pubkey(), &mint.pubkey()),
            spl_token_interface::instruction::mint_to(
                &token_program,
                &mint.pubkey(),
                &user_ata,
                &payer,
                &[],
                USER_TOKENS,
            )?,
            spl_token_interface::instruction::approve(
                &token_program,
                &user_ata,
                &escrow_authority,
                &user.pubkey(),
                &[],
                USER_TOKENS,
            )?,
        ],
        &[&user],
    )
    .await
    .context("failed to fund the user")?;

    let submitter = Arc::new(SplTokenForwarder::new(program, payer));
    env.protocol_adapter
        .forwarders
        .register(program, submitter.clone());
    env.protocol_adapter
        .extend_lookup_table(vec![
            program,
            derive_forwarder_config_pda(&program).0,
            anoma_pa_solana_client::derive_event_authority_pda(&program).0,
            escrow_authority,
            token_program,
            INSTRUCTIONS_SYSVAR_ID,
            derive_associated_token_address(&escrow_authority, &mint.pubkey()),
        ])
        .await?;

    Ok(Forwarder {
        submitter,
        program,
        mint: mint.pubkey(),
        user,
    })
}

/// The token balance of `owner`'s account for `mint`.
pub async fn balance<P>(
    env: &Environment<P>,
    owner: &Pubkey,
    mint: &Pubkey,
) -> anyhow::Result<u64> {
    let account = derive_associated_token_address(owner, mint);
    let amount = env
        .protocol_adapter
        .rpc
        .get_token_account_balance(&account)
        .await
        .with_context(|| format!("failed to read the token account {account}"))?
        .amount;
    amount
        .parse()
        .with_context(|| format!("the token account {account} holds {amount}"))
}
