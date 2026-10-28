//! The forwarder on one of the adapter harness's environments, as the
//! ERC20 forwarder's tests add theirs to pa-evm's.

use std::sync::Arc;

use anchor_lang::{AccountDeserialize, AnchorDeserialize, AnchorSerialize, Discriminator};
use anoma_pa_solana_client::settlement::CallAccounts;
use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_solana_integration_test::executed::Executed;
use anoma_pa_testkit::environment::Prover;
use anoma_pa_testkit::transaction::Transaction;
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_rm_risc0::resource::Resource;
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::accounts::{
    Config, NonceBitmap,
};
#[cfg(feature = "e2e")]
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::client::args::InitNonceBitmap;
use anomapay_spl_token_forwarder_client::generated::spl_token_forwarder::constants::{
    CONFIG_VERSION, NONCES_PER_WORD,
};
use anomapay_spl_token_forwarder_client::{
    derive_forwarder_config_pda, derive_forwarder_escrow_authority, initialize_ix, sha256,
    upgrade_ixs,
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
use transfer_library::TransferLogic;

use crate::fixtures::{self, ShieldedOwner, WrapTerms};
use crate::logic::Logic;
use crate::submitter::Rewritten;
use anomapay_spl_token_forwarder_client::submitter::SplTokenForwarder;

/// The forwarder builds the local tests load: the deterministic builds at
/// the local addresses (`dev.sh test-program`), production and development.
pub const FORWARDER_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder.so");
const FORWARDER_DEV_SO: &[u8] = include_bytes!("../programs/spl_token_forwarder_dev.so");
/// The emergency-migrating build (`dev.sh test-program`), to which an owner
/// upgrades the forwarder after a flaw in a circuit version.
pub const FORWARDER_EMERGENCY_MIGRATING_SO: &[u8] =
    include_bytes!("../programs/spl_token_forwarder_emergency_migrating.so");
/// The emergency-migrating build with the development features
/// (`dev_set_config_version`), with which a test runs `reinitialize` again
/// as a later implementation's.
pub const FORWARDER_EMERGENCY_MIGRATING_DEV_SO: &[u8] =
    include_bytes!("../programs/spl_token_forwarder_emergency_migrating_dev.so");
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

/// The forwarder as a test deploys it: also the owner it was initialized
/// with.
pub struct OwnedForwarder {
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
    };
    env.deploy_program(program, so, env.protocol_adapter.payer_pubkey())?;
    Ok(program)
}

/// Initializes the forwarder `program` on `env` for the adapter and the
/// transfer resource's logic ref, with a new owner, which it returns.
async fn initialize<P>(env: &Environment<P>, program: Pubkey) -> anyhow::Result<Keypair> {
    let payer = env.protocol_adapter.payer_pubkey();
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
pub async fn local() -> anyhow::Result<(LocalEnv, OwnedForwarder)> {
    local_with(Build::Production).await
}

/// `local`, with the forwarder `build`.
async fn local_with(build: Build) -> anyhow::Result<(LocalEnv, OwnedForwarder)> {
    let mut env = LocalEnv::setup_bare().await?;
    let owned = owned(&mut env, build).await?;
    Ok((env, owned))
}

/// Deploys the forwarder `build` on `env`, initializes it for the adapter
/// and the transfer resource's logic ref with a new owner, and has it serve
/// a new mint.
async fn owned<P>(env: &mut Environment<P>, build: Build) -> anyhow::Result<OwnedForwarder> {
    let program = deploy(env, build)?;
    let owner = initialize(env, program).await?;
    let forwarder = serve(env, program).await?;
    Ok(OwnedForwarder { forwarder, owner })
}

/// `local`, upgraded in place by the owner to `build`, the emergency-migrating
/// build (`FORWARDER_EMERGENCY_MIGRATING_SO`) or its development build, as
/// after a flaw in a circuit version of the transfer resource; its
/// `reinitialize` has not run.
pub async fn upgraded_to(build: &[u8]) -> anyhow::Result<(LocalEnv, OwnedForwarder)> {
    let (env, local) = local().await?;
    upgrade(&env, &local.forwarder.program, &local.owner, build)
        .await
        .context("the owner failed to upgrade the forwarder")?;
    Ok((env, local))
}

/// The forwarder `program`'s owner upgrades it in place to `code`, through
/// its `upgrade`, the owner writing and paying for the buffer. The program
/// data first grows to hold `code`, as `ops.sh upgrade` extends it: by the
/// shortfall, at least the loader's minimum extension.
pub async fn upgrade<P>(
    env: &Environment<P>,
    program: &Pubkey,
    owner: &Keypair,
    code: &[u8],
) -> anyhow::Result<()> {
    give_sol(env, &owner.pubkey(), 1_000_000_000)?;
    let program_data = env
        .protocol_adapter
        .rpc
        .get_account_data(&anoma_pa_solana_client::derive_program_data_address(
            program,
        ))
        .await
        .with_context(|| format!("{program} has no program data"))?;
    let held = program_data.len() - UpgradeableLoaderState::size_of_programdata_metadata();
    if code.len() > held {
        let shortfall = u32::try_from(code.len() - held)?
            .max(solana_loader_v3_interface::instruction::MINIMUM_EXTEND_PROGRAM_BYTES);
        env.send(
            &[solana_loader_v3_interface::instruction::extend_program(
                program,
                Some(&owner.pubkey()),
                shortfall,
            )],
            &[owner],
        )
        .await
        .context("failed to extend the program data")?;
    }
    let buffer = env.write_buffer(code, owner.pubkey()).await?;
    env.send(
        &upgrade_ixs(program, &owner.pubkey(), &buffer, &owner.pubkey()),
        &[owner],
    )
    .await?;
    Ok(())
}

/// The e2e environment (`E2E_RUNTIME`), and the runtime it runs on.
#[cfg(feature = "e2e")]
async fn e2e_environment() -> anyhow::Result<(
    anoma_pa_solana_integration_test::envs::e2e::Environment,
    anoma_pa_solana_integration_test::envs::e2e::E2eRuntime,
)> {
    use anoma_pa_solana_integration_test::envs::e2e::{E2eConfig, Environment};

    let e2e_config = E2eConfig::from_env().context("failed to parse e2e test config")?;
    let env = Environment::setup(&e2e_config).await?;
    Ok((env, e2e_config.runtime))
}

/// The e2e environment (`E2E_RUNTIME`) with the forwarder: on its own
/// runtime, the forwarder deployed and initialized for the adapter and the
/// transfer resource's logic ref, as the local environment's; on a fork of
/// devnet or on devnet itself, the forwarder devnet runs, checked to serve
/// the devnet adapter and the transfer resource's logic ref.
#[cfg(feature = "e2e")]
pub async fn e2e() -> anyhow::Result<(
    anoma_pa_solana_integration_test::envs::e2e::Environment,
    Forwarder,
)> {
    use anoma_pa_solana_integration_test::envs::e2e::E2eRuntime;

    let (mut env, runtime) = e2e_environment().await?;
    let forwarder = match runtime {
        E2eRuntime::Local => owned(&mut env, Build::Production).await?.forwarder,
        E2eRuntime::Devnet { .. } | E2eRuntime::Cluster { .. } => {
            let program = forwarder_address(DEVNET)?;
            let config = config(&env, &program).await?;
            let pa = env.protocol_adapter.program;
            anyhow::ensure!(
                config.protocol_adapter == pa,
                "the devnet forwarder serves the adapter {}, not {pa}",
                config.protocol_adapter
            );
            anyhow::ensure!(
                config.logic_ref == <[u8; 32]>::from(crate::logic::logic_ref()),
                "the devnet forwarder serves the logic ref {}, the tests build {}",
                anoma_rm_risc0::Digest::from_bytes(config.logic_ref),
                crate::logic::logic_ref()
            );
            // The settlement of a wrap whose word has no bitmap carries the
            // forwarder's `init_nonce_bitmap`, which a live cluster refuses
            // unless allowed.
            env.protocol_adapter
                .allow_on_live_cluster(program, InitNonceBitmap::DISCRIMINATOR)?;
            serve(&mut env, program).await?
        }
    };
    Ok((env, forwarder))
}

/// The e2e environment (`E2E_RUNTIME`) with the production forwarder
/// deployed at its local address and initialized by a test owner, on its own
/// runtime or a fork of devnet: a test of the owner's operations (an
/// upgrade, `reinitialize`) needs the owner's key, which devnet's
/// forwarder's owner keeps. The cluster runtime, where nothing is deployed
/// but by the operator, is refused.
#[cfg(feature = "e2e")]
pub async fn e2e_owned() -> anyhow::Result<(
    anoma_pa_solana_integration_test::envs::e2e::Environment,
    OwnedForwarder,
)> {
    use anoma_pa_solana_integration_test::envs::e2e::E2eRuntime;

    let (mut env, runtime) = e2e_environment().await?;
    anyhow::ensure!(
        !matches!(runtime, E2eRuntime::Cluster { .. }),
        "E2E_RUNTIME is cluster: an owner's operations deploy a forwarder of their own, which \
         only a runtime of the tests' own or a fork of devnet takes"
    );
    let owned = owned(&mut env, Build::Production).await?;
    Ok((env, owned))
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
    let payer = env.protocol_adapter.payer_pubkey();
    let user = Keypair::new();
    fund(env, &program, &mint, &user, USER_TOKENS).await?;

    let submitter = Arc::new(SplTokenForwarder::new(
        program,
        env.protocol_adapter.program,
        payer,
    ));
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
    let payer = env.protocol_adapter.payer_pubkey();
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
    let payer = env.protocol_adapter.payer_pubkey();
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
    let payer = env.protocol_adapter.payer_pubkey();
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
    env.surfnet()?
        .cheatcodes()
        .fund_sol(to, lamports)
        .with_context(|| format!("failed to fund {to} with {lamports} lamports"))
}

/// Whether two values of a generated type, which derives no `PartialEq`,
/// are equal: whether their Borsh encodings are.
pub fn same<T: AnchorSerialize>(a: &T, b: &T) -> bool {
    let encoding = |value: &T| {
        let mut bytes = Vec::new();
        value
            .serialize(&mut bytes)
            .expect("a Vec takes any Borsh encoding");
        bytes
    };
    encoding(a) == encoding(b)
}

/// The config `initialize` stores on `env` for the adapter, the transfer
/// resource's logic ref and `owner`.
pub fn expected_config<P>(env: &Environment<P>, owner: &Pubkey) -> Config {
    Config {
        protocol_adapter: env.protocol_adapter.program,
        logic_ref: crate::logic::logic_ref().into(),
        version: CONFIG_VERSION,
        owner: *owner,
    }
}

/// The forwarder `program`'s config on `env`.
pub async fn config<P>(env: &Environment<P>, program: &Pubkey) -> anyhow::Result<Config> {
    let (config, _) = derive_forwarder_config_pda(program);
    let data = env
        .protocol_adapter
        .rpc
        .get_account_data(&config)
        .await
        .with_context(|| format!("the forwarder {program} has no config {config}"))?;
    Config::try_deserialize(&mut data.as_slice())
        .with_context(|| format!("the account {config} of {program} is not a config"))
}

/// `local` with the development build, whose config the owner then puts
/// (`dev_set_config_version`) one below this build's CONFIG_VERSION, as an
/// earlier build would have left it: the config `reinitialize` rotates once.
pub async fn local_below_this_builds_version() -> anyhow::Result<(LocalEnv, OwnedForwarder)> {
    let (env, local) = local_with(Build::Development).await?;
    set_config_version(&env, &local, CONFIG_VERSION - 1).await?;
    Ok((env, local))
}

/// The owner sets the config's version to `version` with the development
/// builds' `dev_set_config_version`.
pub async fn set_config_version<P>(
    env: &Environment<P>,
    local: &OwnedForwarder,
    version: u64,
) -> anyhow::Result<()> {
    let (program, owner) = (local.forwarder.program, local.owner.pubkey());
    // The development builds' instruction, which the release IDLs leave
    // out: Anchor's discriminator of it.
    let mut data = sha256(b"global:dev_set_config_version")[..8].to_vec();
    data.extend_from_slice(&version.to_le_bytes());
    let set = Instruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new(derive_forwarder_config_pda(&program).0, false),
        ],
        data,
    };
    env.send(&[set], &[&local.owner])
        .await
        .context("failed to set the config's version")?;
    Ok(())
}

/// The nonce bitmap at `address` on `env`.
pub async fn nonce_bitmap<P>(
    env: &Environment<P>,
    address: &Pubkey,
) -> anyhow::Result<NonceBitmap> {
    let data = env
        .protocol_adapter
        .rpc
        .get_account_data(address)
        .await
        .with_context(|| format!("there is no nonce bitmap {address}"))?;
    NonceBitmap::try_deserialize(&mut data.as_slice())
        .with_context(|| format!("the account {address} is not a nonce bitmap"))
}

/// Whether a wrap has used `nonce`, which falls in `bitmap`'s word: the
/// program's bit `nonce % NONCES_PER_WORD`, least significant first.
pub fn nonce_is_used(bitmap: &NonceBitmap, nonce: u64) -> bool {
    let bit = nonce % NONCES_PER_WORD;
    bitmap.bits[(bit / 8) as usize] & (1 << (bit % 8)) != 0
}

/// The discriminators of the forwarder `program`'s events in the
/// transaction `executed`, in the order it emitted them: the order the
/// typed reads (`Executed::events`) do not keep across event types.
pub fn event_order(executed: &Executed, program: &Pubkey) -> anyhow::Result<Vec<[u8; 8]>> {
    Ok(executed
        .cpi_events(program)
        .map(anoma_pa_solana_client::cpi_event_discriminator)
        .collect::<Result<_, _>>()?)
}

/// The event `E` the forwarder `program` emitted in the transaction
/// `executed`, its only event.
pub fn sole_event<E: Discriminator + AnchorDeserialize>(
    executed: &Executed,
    program: &Pubkey,
) -> anyhow::Result<E> {
    let order = event_order(executed, program)?;
    anyhow::ensure!(
        order == [discriminator::<E>()],
        "{program} emitted the events {order:02x?}, not one {}",
        std::any::type_name::<E>()
    );
    Ok(executed.events::<E>(program)?.remove(0))
}

/// The discriminator of the event `E`, as `event_order` lists it.
pub fn discriminator<E: Discriminator>() -> [u8; 8] {
    E::DISCRIMINATOR
        .try_into()
        .expect("an event discriminator is 8 bytes")
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

/// A proven migration and the resource of the emergency-migrating logic it
/// creates.
pub struct ProvenMigration {
    pub tx: Transaction,
    pub created: Resource,
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

    /// An unwrap of `wrapped`, a resource of the logic `L` that `owner`
    /// holds in the adapter's commitment tree, to `recipient`'s token
    /// account, proven on `env`.
    pub async fn prove_unwrap<L: Logic, P: Prover>(
        &self,
        env: &Environment<P>,
        wrapped: Resource,
        owner: &ShieldedOwner,
        recipient: Pubkey,
    ) -> anyhow::Result<Transaction> {
        let path = env
            .protocol_adapter
            .commitment_tree
            .path_to(wrapped.commitment())?;
        let unwrap =
            fixtures::unwrap::<L>(self.program, self.mint, wrapped, owner, recipient, path)?;
        prove_actions(env, &[unwrap]).await
    }

    /// `owner` unwraps `wrapped`, a resource of the logic `L` holding
    /// `amount`, to a fresh recipient, settled on `env`: the escrow releases
    /// `amount` to the recipient's token account.
    pub async fn unwraps<L: Logic, P: Prover>(
        &self,
        env: &mut Environment<P>,
        wrapped: Resource,
        owner: &ShieldedOwner,
        amount: u64,
    ) -> anyhow::Result<()> {
        let escrow = self.escrow_account();
        let escrow_before = balance(env, &escrow).await?;
        let recipient = Keypair::new().pubkey();
        let tx = self
            .prove_unwrap::<L, P>(env, wrapped, owner, recipient)
            .await?;
        execute_tx(env, tx).await?;
        let received = balance(env, &get_associated_token_address(&recipient, &self.mint)).await?;
        anyhow::ensure!(
            received == amount,
            "the unwrap released {received} to the recipient, not {amount}"
        );
        let escrow_after = balance(env, &escrow).await?;
        anyhow::ensure!(
            escrow_after == escrow_before - amount,
            "the escrow holds {escrow_after} after the unwrap, not {escrow_before} - {amount}"
        );
        Ok(())
    }

    /// The user wraps 100 tokens into a resource a shielded owner holds, then
    /// the owner unwraps them to a fresh recipient, both settled on `env`:
    /// the user's tokens move into escrow, then out to the recipient.
    pub async fn wraps_then_unwraps<P>(&self, env: &mut Environment<P>) -> anyhow::Result<()>
    where
        P: Prover,
    {
        // 100 tokens at the mint's 6 decimals.
        let amount = 100_000_000;
        let seed = "wrap-then-unwrap";
        let owner = ShieldedOwner::seeded(&format!("{seed}/owner"));
        let (user, escrow) = (self.user_account(), self.escrow_account());
        let user_before = balance(env, &user).await?;
        let escrow_before = balance(env, &escrow).await?;

        let wrapped = self
            .wrap(env, &owner, amount, 1, &format!("{seed}/wrap"))
            .await?;
        anyhow::ensure!(balance(env, &user).await? == user_before - amount);
        anyhow::ensure!(balance(env, &escrow).await? == escrow_before + amount);

        self.unwraps::<TransferLogic, P>(env, wrapped, &owner, amount)
            .await
    }

    /// A migration of `wrapped`, a resource under this forwarder's label that
    /// `owner` holds in the adapter's commitment tree, into a resource of the
    /// emergency-migrating logic `owner` holds, proven on `env`. `seed` makes
    /// its resources distinct from every other migration's.
    pub async fn prove_migration<P: Prover>(
        &self,
        env: &Environment<P>,
        wrapped: Resource,
        owner: &ShieldedOwner,
        seed: &str,
    ) -> anyhow::Result<ProvenMigration> {
        let path = env
            .protocol_adapter
            .commitment_tree
            .path_to(wrapped.commitment())?;
        let migration = fixtures::migrate(self.program, self.mint, wrapped, path, owner, seed)?;
        Ok(ProvenMigration {
            tx: prove_actions(env, &[migration.witnesses]).await?,
            created: migration.created,
        })
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
