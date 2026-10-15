//! The forwarder on one of the adapter harness's environments, as the
//! ERC20 forwarder's tests add theirs to pa-evm's.

use std::sync::Arc;

use anoma_pa_solana_client::anchor_instruction_disc;
use anoma_pa_solana_client::settlement::CallAccounts;
use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_rm_risc0::resource::Resource;
use anomapay_spl_token_forwarder_client::{
    CONFIG_VERSION, ConfigAccount, ForwarderEvent, decode_config,
    decode_forwarder_event_instruction, derive_forwarder_config_pda,
    derive_forwarder_escrow_authority, initialize_ix, sha256, upgrade_ixs,
};
use anyhow::Context;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use solana_program_pack::Pack;
use solana_signer::Signer;
use spl_associated_token_account_interface::address::get_associated_token_address;
use spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent;
use surfpool_sdk::Pubkey;

use crate::fixtures::{self, ShieldedOwner, WrapTerms};
use crate::submitter::Rewritten;
use anomapay_spl_token_forwarder_client::submitter::SplTokenForwarder;

/// The forwarder builds the local tests load: the deterministic builds at
/// the local addresses (`dev.sh test-program`), production and development.
pub const FORWARDER_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder.so");
const FORWARDER_DEV_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder_dev.so");
/// The previous release's production build, whose config held an emergency
/// committee and caller: where an in-place upgrade to this release starts.
const PREVIOUS_FORWARDER_SO: &[u8] = include_bytes!("../programs/previous/spl_token_forwarder.so");
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

/// The forwarder as the local environment deploys it: also the owner it was
/// initialized with.
pub struct LocalForwarder {
    pub forwarder: Forwarder,
    pub owner: Keypair,
}

/// Which build of the forwarder a local environment deploys.
#[derive(Clone, Copy)]
pub enum Build {
    Production,
    /// With the development features' instructions
    /// (`dev_set_config_version`).
    Development,
    /// The previous release's production build.
    Previous,
}

/// The local environment with the forwarder `build` deployed, its upgrade
/// authority the adapter's payer, and not initialized; and the forwarder's
/// address.
pub async fn deployed(build: Build) -> anyhow::Result<(LocalEnv, Pubkey)> {
    let env = LocalEnv::setup_bare().await?;
    let program = deploy(&env, build)?;
    Ok((env, program))
}

/// Deploys the forwarder `build` on `env` at its local address, its upgrade
/// authority the adapter's payer, and not initialized; returns the address.
fn deploy<P>(env: &Environment<P>, build: Build) -> anyhow::Result<Pubkey> {
    let program = forwarder_address(LOCALNET)?;
    let so = match build {
        Build::Production => FORWARDER_SO,
        Build::Development => FORWARDER_DEV_SO,
        Build::Previous => PREVIOUS_FORWARDER_SO,
    };
    env.deploy_program(program, so, env.protocol_adapter.payer.pubkey())?;
    Ok(program)
}

/// Initializes the forwarder `program` on `env` for the adapter and the
/// transfer resource's logic ref, with a new owner, which it returns.
async fn initialize<P>(env: &Environment<P>, program: Pubkey) -> anyhow::Result<Keypair> {
    let payer = env.protocol_adapter.payer.pubkey();
    let owner = Keypair::new();
    env.send(
        &[initialize_ix(
            &program,
            &payer,
            &env.protocol_adapter.program,
            crate::logic::logic_ref().into(),
            &owner.pubkey(),
        )],
        &[],
    )
    .await
    .context("failed to initialize the forwarder")?;
    Ok(owner)
}

/// The local environment with the forwarder deployed and initialized for the
/// adapter and the transfer resource's logic ref.
pub async fn local() -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    local_with(Build::Production).await
}

/// `local`, with the forwarder `build`.
async fn local_with(build: Build) -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    let (mut env, program) = deployed(build).await?;
    let owner = initialize(&env, program).await?;
    let forwarder = serve(&mut env, program).await?;
    Ok((env, LocalForwarder { forwarder, owner }))
}

/// The local environment with the previous release deployed, initialized
/// for the adapter and the transfer resource's logic ref with a new owner
/// and emergency committee, and serving a mint; then upgraded in place by
/// its owner to this release's production build, as a cluster's forwarder
/// is. The config is still in the previous layout: `migrate_config` has not
/// run.
pub async fn upgraded_from_the_previous_release() -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    let (mut env, program) = deployed(Build::Previous).await?;
    let payer = env.protocol_adapter.payer.pubkey();

    // The previous release's initialize took the committee between the
    // logic ref and the owner, over the same accounts.
    let owner = Keypair::new();
    let mut initialize = initialize_ix(
        &program,
        &payer,
        &env.protocol_adapter.program,
        crate::logic::logic_ref().into(),
        &owner.pubkey(),
    );
    initialize.data = [
        anchor_instruction_disc("initialize").as_slice(),
        env.protocol_adapter.program.as_ref(),
        &<[u8; 32]>::from(crate::logic::logic_ref()),
        Keypair::new().pubkey().as_ref(),
        owner.pubkey().as_ref(),
    ]
    .concat();
    env.send(&[initialize], &[])
        .await
        .context("failed to initialize the previous release")?;
    let forwarder = serve(&mut env, program).await?;

    give_sol(&env, &owner.pubkey(), 1_000_000_000)?;
    let buffer = env.write_buffer(FORWARDER_SO, owner.pubkey()).await?;
    env.send(
        &upgrade_ixs(&program, &owner.pubkey(), &buffer, &owner.pubkey()),
        &[&owner],
    )
    .await
    .context("the owner failed to upgrade the previous release")?;
    Ok((env, LocalForwarder { forwarder, owner }))
}

/// The e2e environment (`E2E_RUNTIME`) with the forwarder: on its own
/// runtime, the forwarder deployed and initialized for the adapter and the
/// transfer resource's logic ref, as the local environment's; on a fork of
/// devnet, the forwarder devnet runs, checked to serve the devnet adapter and
/// the transfer resource's logic ref.
#[cfg(feature = "e2e")]
pub async fn e2e() -> anyhow::Result<(
    anoma_pa_solana_integration_test::envs::e2e::Environment,
    Forwarder,
)> {
    use anoma_pa_solana_integration_test::envs::e2e::{E2eConfig, E2eRuntime, Environment};

    let e2e_config = E2eConfig::from_env().context("failed to parse e2e test config")?;
    let mut env = Environment::setup(&e2e_config).await?;
    let program = match e2e_config.runtime {
        E2eRuntime::Local => {
            let program = deploy(&env, Build::Production)?;
            initialize(&env, program).await?;
            program
        }
        E2eRuntime::Devnet { .. } => {
            let program = forwarder_address(DEVNET)?;
            let config = config(&env, &program).await?;
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
            program
        }
    };
    let forwarder = serve(&mut env, program).await?;
    Ok((env, forwarder))
}

/// Has the forwarder `program` serve a new mint on `env`: the mint, its
/// escrow, a user holding `USER_TOKENS` who has approved the escrow authority
/// to move them, and the submitter registered with the adapter.
async fn serve<P>(env: &mut Environment<P>, program: Pubkey) -> anyhow::Result<Forwarder> {
    let mint = create_mint(env, &program).await?;
    serve_mint(env, program, mint).await
}

/// Has the forwarder `program` serve `mint`: a user holding `USER_TOKENS`
/// who has approved the escrow authority to move them, and the submitter
/// registered with the adapter.
async fn serve_mint<P>(
    env: &mut Environment<P>,
    program: Pubkey,
    mint: Pubkey,
) -> anyhow::Result<Forwarder> {
    let payer = env.protocol_adapter.payer.pubkey();
    let user = Keypair::new();
    fund(env, &program, &mint, &user, USER_TOKENS).await?;

    let submitter = Arc::new(SplTokenForwarder::new(program, payer));
    env.protocol_adapter
        .forwarders
        .register(program, submitter.clone());

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
    escrow(env, program, &mint).await?;
    Ok(mint)
}

/// The forwarder `program`'s escrow account for `mint`.
async fn escrow<P>(env: &Environment<P>, program: &Pubkey, mint: &Pubkey) -> anyhow::Result<()> {
    let (escrow_authority, _) = derive_forwarder_escrow_authority(program);
    token_account(env, &escrow_authority, mint)
        .await
        .context("failed to create the mint's escrow")?;
    Ok(())
}

/// `owner`'s token account for `mint`, created if it does not exist.
pub async fn token_account<P>(
    env: &Environment<P>,
    owner: &Pubkey,
    mint: &Pubkey,
) -> anyhow::Result<Pubkey> {
    let payer = env.protocol_adapter.payer.pubkey();
    env.send(
        &[create_associated_token_account_idempotent(
            &payer,
            owner,
            mint,
            &spl_token_interface::id(),
        )],
        &[],
    )
    .await
    .with_context(|| format!("failed to create {owner}'s token account for {mint}"))?;
    Ok(get_associated_token_address(owner, mint))
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

/// Sets `to`'s balance to `lamports`: what a new signer that pays for an
/// account needs.
pub fn give_sol<P>(env: &Environment<P>, to: &Pubkey, lamports: u64) -> anyhow::Result<()> {
    env.surfnet
        .cheatcodes()
        .fund_sol(to, lamports)
        .with_context(|| format!("failed to fund {to} with {lamports} lamports"))
}

/// The config `initialize` stores on `env` for the adapter, the transfer
/// resource's logic ref and `owner`.
pub fn expected_config<P>(env: &Environment<P>, owner: &Pubkey) -> ConfigAccount {
    ConfigAccount {
        protocol_adapter: env.protocol_adapter.program.to_bytes(),
        logic_ref: crate::logic::logic_ref().into(),
        version: CONFIG_VERSION,
        owner: owner.to_bytes(),
    }
}

/// The forwarder `program`'s config on `env`.
pub async fn config<P>(env: &Environment<P>, program: &Pubkey) -> anyhow::Result<ConfigAccount> {
    let (config, _) = derive_forwarder_config_pda(program);
    let data = env
        .protocol_adapter
        .rpc
        .get_account_data(&config)
        .await
        .with_context(|| format!("the forwarder {program} has no config {config}"))?;
    Ok(decode_config(&data)?)
}

/// `local` with the development build, whose config the owner then puts
/// (`dev_set_config_version`) one below this build's CONFIG_VERSION, as an
/// earlier build would have left it: the config `reinitialize` rotates once.
pub async fn local_below_this_builds_version() -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    let (env, local) = local_with(Build::Development).await?;
    let (program, owner) = (local.forwarder.program, local.owner.pubkey());
    let mut data = anchor_instruction_disc("dev_set_config_version").to_vec();
    data.extend_from_slice(&(CONFIG_VERSION - 1).to_le_bytes());
    let lower = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new(derive_forwarder_config_pda(&program).0, false),
        ],
        data,
    };
    env.send(&[lower], &[&local.owner])
        .await
        .context("failed to lower the config's version")?;
    Ok((env, local))
}

/// The forwarder `program`'s events in the transaction `executed`, in the
/// order it emitted them.
pub fn events(executed: &Executed, program: &Pubkey) -> anyhow::Result<Vec<ForwarderEvent>> {
    Ok(executed
        .cpi_events(program)
        .map(decode_forwarder_event_instruction)
        .collect::<Result<_, _>>()?)
}

/// The executable hash of a program's code, as `solana-verify
/// get-executable-hash` computes it: the sha256 of the code without its
/// trailing zero bytes.
pub fn executable_hash(code: &[u8]) -> [u8; 32] {
    let end = code.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    sha256(&code[..end])
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

/// A wrap, proven: the transaction, which carries the user's signature in the
/// wrap's input, the shielded resource it creates, and the message the user
/// signed.
pub struct ProvenWrap {
    pub tx: Transaction,
    pub created: Resource,
    /// The base64 digest of the wrap message, as UTF-8 text.
    pub signed_message: Vec<u8>,
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
        P: Prover,
    {
        let wrap = self.wrap_data(owner, amount, nonce, 0, seed)?;
        Ok(ProvenWrap {
            tx: prove_actions(env, &[wrap.witnesses]).await?,
            created: wrap.created,
            signed_message: wrap.signed_message,
        })
    }

    /// Wraps of `wraps`' amounts under their forwarder nonces, each from the
    /// user into a resource `owner` holds, in one transaction proven on
    /// `env`. `seed` makes their resources distinct from every other wrap's.
    pub async fn prove_wraps<P>(
        &self,
        env: &Environment<P>,
        owner: &ShieldedOwner,
        wraps: &[(u64, u64)],
        seed: &str,
    ) -> anyhow::Result<Transaction>
    where
        P: Prover,
    {
        let mut witnesses = Vec::new();
        for (i, &(amount, nonce)) in wraps.iter().enumerate() {
            let wrap = self.wrap_data(
                owner,
                amount,
                nonce,
                u8::try_from(i)?,
                &format!("{seed}/{i}"),
            )?;
            witnesses.push(wrap.witnesses);
        }
        prove_actions(env, &witnesses).await
    }

    /// A wrap's witnesses, signed by the user, whose signature check is the
    /// transaction's instruction `ed25519_ix_index`: the submitter puts a
    /// settlement's checks first, in call order.
    fn wrap_data(
        &self,
        owner: &ShieldedOwner,
        amount: u64,
        nonce: u64,
        ed25519_ix_index: u8,
        seed: &str,
    ) -> anyhow::Result<fixtures::WrapData> {
        fixtures::wrap(
            self.program,
            self.mint,
            &self.user,
            owner,
            WrapTerms {
                amount,
                nonce,
                ed25519_ix_index,
            },
            anoma_pa_testkit::fixtures::kind_table(),
            seed,
        )
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
        P: Prover,
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
        P: Prover,
    {
        let path = env
            .protocol_adapter
            .commitment_tree
            .path_to(wrapped.commitment())?;
        let unwrap = fixtures::unwrap(self.program, self.mint, wrapped, owner, recipient, path)?;
        prove_actions(env, &[unwrap]).await
    }

    /// The user wraps `amount` into a resource a shielded owner holds, then
    /// the owner unwraps it to a fresh recipient, both settled on `env`: the
    /// user's tokens move into escrow, then out to the recipient. `seed`
    /// makes the resources distinct from every other wrap's.
    pub async fn wraps_then_unwraps<P>(
        &self,
        env: &mut Environment<P>,
        amount: u64,
        seed: &str,
    ) -> anyhow::Result<()>
    where
        P: Prover,
    {
        let owner = ShieldedOwner::seeded(&format!("{seed}/owner"));
        let (user, escrow) = (self.user_account(), self.escrow_account());
        let user_before = balance(env, &user).await?;
        let escrow_before = balance(env, &escrow).await?;

        let wrapped = self
            .wrap(env, &owner, amount, 1, &format!("{seed}/wrap"))
            .await?;
        anyhow::ensure!(balance(env, &user).await? == user_before - amount);
        anyhow::ensure!(balance(env, &escrow).await? == escrow_before + amount);

        let recipient = Keypair::new().pubkey();
        let tx = self.prove_unwrap(env, wrapped, &owner, recipient).await?;
        execute_tx(env, tx).await?;
        let recipient_account = get_associated_token_address(&recipient, &self.mint);
        anyhow::ensure!(balance(env, &recipient_account).await? == amount);
        anyhow::ensure!(balance(env, &escrow).await? == escrow_before);
        Ok(())
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
        get_associated_token_address(&self.user.pubkey(), &self.mint)
    }

    /// The escrow's token account for the mint.
    pub fn escrow_account(&self) -> Pubkey {
        let (escrow_authority, _) = derive_forwarder_escrow_authority(&self.program);
        get_associated_token_address(&escrow_authority, &self.mint)
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
