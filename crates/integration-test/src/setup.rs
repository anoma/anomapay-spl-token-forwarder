//! The forwarder on one of the adapter harness's environments, as the
//! ERC20 forwarder's tests add theirs to pa-evm's.

use std::sync::Arc;

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::forwarders::CallAccounts;
use anoma_pa_testkit::environment::{CommitmentTree as _, Prover};
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_rm_risc0::resource::Resource;
use anomapay_spl_token_forwarder_client::{
    INSTRUCTIONS_SYSVAR_ID, create_ata_idempotent_ix, derive_associated_token_address,
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, initialize_ix,
};
use anyhow::Context;
use solana_keypair::Keypair;
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use solana_program_pack::Pack;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

use crate::fixtures::{self, ShieldedOwner, WrapAuthorization, WrapTerms};
use crate::submitter::{Rewritten, SplTokenForwarder};

/// The forwarder builds the local tests load: the deterministic builds at
/// the local addresses (`dev.sh test-program`), production and development.
pub const FORWARDER_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder.so");
const FORWARDER_DEV_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder_dev.so");
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

/// Which build of the forwarder a local environment deploys.
#[derive(Clone, Copy)]
pub enum Build {
    Production,
    /// With the development features' instructions
    /// (`dev_set_config_version`).
    Development,
}

/// The local environment with the forwarder `build` deployed, its upgrade
/// authority the adapter's payer, and not initialized; and the forwarder's
/// address.
pub async fn deployed(build: Build) -> anyhow::Result<(LocalEnv, Pubkey)> {
    let env = LocalEnv::setup_bare().await?;
    let program = forwarder_address(LOCALNET)?;
    let so = match build {
        Build::Production => FORWARDER_SO,
        Build::Development => FORWARDER_DEV_SO,
    };
    env.deploy_program(program, so, env.protocol_adapter.payer.pubkey())?;
    Ok((env, program))
}

/// The local environment with the forwarder deployed and initialized for the
/// adapter and the transfer resource's logic ref.
pub async fn local() -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    local_with(Build::Production).await
}

/// `local`, with the forwarder `build`.
pub async fn local_with(build: Build) -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    let (mut env, program) = deployed(build).await?;
    let payer = env.protocol_adapter.payer.pubkey();
    let (owner, committee) = (Keypair::new(), Keypair::new());
    env.send(
        &[initialize_ix(
            &program,
            &payer,
            &env.protocol_adapter.program,
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
    let pa = env.protocol_adapter.program;
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
    let payer = env.protocol_adapter.payer.pubkey();
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let mint = create_mint(env, &program).await?;
    let user = Keypair::new();
    fund(env, &program, &mint, &user, USER_TOKENS).await?;

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
            spl_token_interface::id(),
            INSTRUCTIONS_SYSVAR_ID,
            derive_associated_token_address(&escrow_authority, &mint),
        ])
        .await?;

    Ok(Forwarder {
        submitter,
        program,
        mint,
        user,
    })
}

/// A new mint of 6 decimals whose authority is the adapter's payer.
pub async fn new_mint<P>(env: &Environment<P>) -> anyhow::Result<Pubkey> {
    let payer = env.protocol_adapter.payer.pubkey();
    let mint = Keypair::new();
    let space = spl_token_interface::state::Mint::LEN;
    let rent = env
        .protocol_adapter
        .rpc
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
        ],
        &[&mint],
    )
    .await
    .context("failed to create the mint")?;
    Ok(mint.pubkey())
}

/// `new_mint`, and the forwarder `program`'s escrow account for it.
pub async fn create_mint<P>(env: &Environment<P>, program: &Pubkey) -> anyhow::Result<Pubkey> {
    let mint = new_mint(env).await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(program);
    token_account(env, &escrow_authority, &mint)
        .await
        .context("failed to create the mint's escrow")?;
    Ok(mint)
}

/// `owner`'s token account for `mint`, created if it does not exist.
pub async fn token_account<P>(
    env: &Environment<P>,
    owner: &Pubkey,
    mint: &Pubkey,
) -> anyhow::Result<Pubkey> {
    let payer = env.protocol_adapter.payer.pubkey();
    env.send(&[create_ata_idempotent_ix(&payer, owner, mint)], &[])
        .await
        .with_context(|| format!("failed to create {owner}'s token account for {mint}"))?;
    Ok(derive_associated_token_address(owner, mint))
}

/// Mints `amount` of `mint`, whose authority is the adapter's payer, to the
/// token account `account`.
pub async fn mint_to<P>(
    env: &Environment<P>,
    mint: &Pubkey,
    account: &Pubkey,
    amount: u64,
) -> anyhow::Result<()> {
    let payer = env.protocol_adapter.payer.pubkey();
    env.send(
        &[spl_token_interface::instruction::mint_to(
            &spl_token_interface::id(),
            mint,
            account,
            &payer,
            &[],
            amount,
        )?],
        &[],
    )
    .await
    .with_context(|| format!("failed to mint {amount} of {mint} to {account}"))?;
    Ok(())
}

/// Gives `owner` `amount` of `mint` in its token account and has `owner`
/// approve the forwarder `program`'s escrow authority to move them: what a
/// wrap from `owner` needs. Returns the token account.
pub async fn fund<P>(
    env: &Environment<P>,
    program: &Pubkey,
    mint: &Pubkey,
    owner: &Keypair,
    amount: u64,
) -> anyhow::Result<Pubkey> {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(program);
    let account = token_account(env, &owner.pubkey(), mint).await?;
    mint_to(env, mint, &account, amount).await?;
    env.send(
        &[spl_token_interface::instruction::approve(
            &spl_token_interface::id(),
            &account,
            &escrow_authority,
            &owner.pubkey(),
            &[],
            amount,
        )?],
        &[owner],
    )
    .await
    .with_context(|| format!("{} failed to approve the escrow authority", owner.pubkey()))?;
    Ok(account)
}

/// Sends `lamports` from the adapter's payer to `to`: what a signer that
/// pays for an account needs.
pub async fn give_sol<P>(env: &Environment<P>, to: &Pubkey, lamports: u64) -> anyhow::Result<()> {
    let payer = env.protocol_adapter.payer.pubkey();
    env.send(
        &[solana_system_interface::instruction::transfer(
            &payer, to, lamports,
        )],
        &[],
    )
    .await
    .with_context(|| format!("failed to send {lamports} lamports to {to}"))?;
    Ok(())
}

/// The upgrade authority the loader records for `program`; None once the
/// program is final.
pub async fn upgrade_authority<P>(
    env: &Environment<P>,
    program: &Pubkey,
) -> anyhow::Result<Option<Pubkey>> {
    let program_data = anoma_pa_solana_client::derive_program_data_address(program);
    let data = env
        .protocol_adapter
        .rpc
        .get_account_data(&program_data)
        .await
        .with_context(|| format!("{program} has no program data {program_data}"))?;
    let metadata = UpgradeableLoaderState::size_of_programdata_metadata();
    match bincode::deserialize(&data[..metadata])
        .with_context(|| format!("failed to decode the program data of {program}"))?
    {
        UpgradeableLoaderState::ProgramData {
            upgrade_authority_address,
            ..
        } => Ok(upgrade_authority_address),
        state => anyhow::bail!("{program_data} holds {state:?}, not program data"),
    }
}

/// A wrap, proven, whose authorization the submitter holds: the
/// transaction, the shielded resource it creates, and what the user signed.
pub struct ProvenWrap {
    pub tx: Transaction,
    pub created: Resource,
    pub authorization: WrapAuthorization,
}

impl Forwarder {
    /// A wrap of `amount` from the user into a resource `owner` holds, under
    /// the forwarder nonce `nonce`, proven on `env`. `seed` makes its
    /// resources distinct from every other wrap's.
    pub async fn prove_wrap<P>(
        &self,
        env: &Environment<P>,
        owner: &ShieldedOwner,
        amount: u64,
        nonce: u64,
        seed: &str,
    ) -> anyhow::Result<ProvenWrap>
    where
        P: Prover<Transaction = Transaction>,
    {
        let wrap = fixtures::wrap(
            self.program,
            self.mint,
            &self.user,
            owner,
            WrapTerms {
                amount,
                nonce,
                // The submitter puts the ed25519 instruction first.
                ed25519_ix_index: 0,
            },
            fixtures::loaded_kind_table(),
            seed,
        )?;
        self.submitter
            .authorize(self.user.pubkey(), nonce, wrap.authorization.clone());
        Ok(ProvenWrap {
            tx: prove_actions(env, &[wrap.witnesses]).await?,
            created: wrap.created,
            authorization: wrap.authorization,
        })
    }

    /// `prove_wrap`, settled: the resource the wrap creates.
    pub async fn wrap<P>(
        &self,
        env: &mut Environment<P>,
        owner: &ShieldedOwner,
        amount: u64,
        nonce: u64,
        seed: &str,
    ) -> anyhow::Result<Resource>
    where
        P: Prover<Transaction = Transaction>,
    {
        let wrap = self.prove_wrap(env, owner, amount, nonce, seed).await?;
        execute_tx(env, wrap.tx).await?;
        Ok(wrap.created)
    }

    /// An unwrap of `wrapped`, a resource `owner` holds in the adapter's
    /// commitment tree, to `recipient`'s token account, proven on `env`.
    pub async fn prove_unwrap<P>(
        &self,
        env: &Environment<P>,
        wrapped: Resource,
        owner: &ShieldedOwner,
        recipient: Pubkey,
    ) -> anyhow::Result<Transaction>
    where
        P: Prover<Transaction = Transaction>,
    {
        let path = env
            .protocol_adapter
            .commitment_tree
            .path_to(wrapped.commitment())?;
        let unwrap = fixtures::unwrap(self.program, self.mint, wrapped, owner, recipient, path)?;
        prove_actions(env, &[unwrap]).await
    }

    /// Has `env`'s adapter pass this forwarder's calls what `rewrite` makes
    /// of the submitter's accounts and instructions, until `restore`.
    pub fn rewrite<P>(
        &self,
        env: &mut Environment<P>,
        rewrite: impl Fn(&mut CallAccounts) + Send + Sync + 'static,
    ) {
        env.protocol_adapter.forwarders.register(
            self.program,
            Arc::new(Rewritten {
                submitter: self.submitter.clone(),
                rewrite,
            }),
        );
    }

    /// Has `env`'s adapter pass this forwarder's calls the submitter's
    /// accounts again.
    pub fn restore<P>(&self, env: &mut Environment<P>) {
        env.protocol_adapter
            .forwarders
            .register(self.program, self.submitter.clone());
    }

    /// The user's token account for the mint.
    pub fn user_account(&self) -> Pubkey {
        derive_associated_token_address(&self.user.pubkey(), &self.mint)
    }

    /// The escrow's token account for the mint.
    pub fn escrow_account(&self) -> Pubkey {
        let (escrow_authority, _) = derive_forwarder_escrow_authority(&self.program);
        derive_associated_token_address(&escrow_authority, &self.mint)
    }
}

/// The token balance of the token account `account`.
pub async fn balance<P>(env: &Environment<P>, account: &Pubkey) -> anyhow::Result<u64> {
    let amount = env
        .protocol_adapter
        .rpc
        .get_token_account_balance(account)
        .await
        .with_context(|| format!("failed to read the token account {account}"))?
        .amount;
    amount
        .parse()
        .with_context(|| format!("the token account {account} holds {amount}"))
}
