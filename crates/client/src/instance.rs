//! The forwarder deployed at an address, as the EVM bindings return the
//! ERC20 forwarder: its reads under their EVM names, from its config and
//! the emergency-migrating build's accounts, and its instructions, built for
//! its address. One type serves each build ([`SplTokenForwarder`],
//! [`EmergencyMigratingSplTokenForwarder`]); the reads and instructions both
//! builds declare are written once, and each build adds its own.

use std::fmt;
use std::marker::PhantomData;

use anchor_lang::{AccountDeserialize, AnchorDeserialize};
use anoma_pa_solana_client::deployments::{chain_id_of_genesis_hash, Environment};
use base64::Engine;
use solana_instruction::Instruction;
use solana_message::{Hash, Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_rpc_client_api::response::{UiReturnDataEncoding, UiTransactionReturnData};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_error::TransactionError;

use crate::accounts::decode_vulnerable_logic_refs;
use crate::deployments::{Deployment, Record, RECORD};
use crate::emergency_migrating::{
    self, derive_migration_record_pda, derive_vulnerable_logic_refs_pda,
};
use crate::forwarder;
use crate::generated::emergency_migrating_spl_token_forwarder::accounts::MigrationRecord;
use crate::generated::emergency_migrating_spl_token_forwarder::constants::LEGACY_FORWARDER;
use crate::generated::spl_token_forwarder::accounts::Config;
use crate::pda::derive_forwarder_config_pda;

/// Why an instance could not be made or could not answer.
#[derive(Debug)]
pub enum InstanceError {
    /// The environment records no deployed forwarder on the cluster.
    NoDeployment { chain_id: String },
    /// An account the answer is read from does not exist.
    AccountMissing { address: Pubkey },
    /// An RPC request failed: retrying may succeed.
    Rpc { what: String, error: String },
    /// What the chain holds does not decode.
    Decode { what: String, error: String },
    /// A simulated instruction failed.
    Simulation {
        error: TransactionError,
        logs: Vec<String>,
    },
}

impl fmt::Display for InstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDeployment { chain_id } => {
                write!(f, "no deployed forwarder is recorded on {chain_id}")
            }
            Self::AccountMissing { address } => write!(f, "the account {address} does not exist"),
            Self::Rpc { what, error } => write!(f, "failed to fetch {what}: {error}"),
            Self::Decode { what, error } => write!(f, "{what} does not decode: {error}"),
            Self::Simulation { error, logs } => {
                write!(f, "the simulation failed: {error}; logs: {logs:?}")
            }
        }
    }
}

impl std::error::Error for InstanceError {}

fn rpc_error(
    what: &str,
) -> impl FnOnce(solana_rpc_client_api::client_error::Error) -> InstanceError + '_ {
    move |error| InstanceError::Rpc {
        what: what.to_owned(),
        error: error.to_string(),
    }
}

/// The forwarder `environment` records on the cluster whose genesis hash is
/// `genesis_hash`. Both builds run at the one address: the
/// emergency-migrating build replaces the production build in place.
pub fn recorded_deployment(
    environment: Environment,
    genesis_hash: &Hash,
) -> Result<Deployment, InstanceError> {
    deployment_in(&RECORD, environment, genesis_hash)
}

fn deployment_in(
    record: &Record,
    environment: Environment,
    genesis_hash: &Hash,
) -> Result<Deployment, InstanceError> {
    let chain_id = chain_id_of_genesis_hash(genesis_hash);
    record
        .deployment(environment, &chain_id)
        .ok_or(InstanceError::NoDeployment { chain_id })
}

/// The program `environment` records on the cluster behind `rpc`.
async fn recorded_program(
    rpc: &RpcClient,
    environment: Environment,
) -> Result<Pubkey, InstanceError> {
    let genesis_hash = rpc
        .get_genesis_hash()
        .await
        .map_err(rpc_error("the genesis hash"))?;
    Ok(recorded_deployment(environment, &genesis_hash)?.program_id)
}

/// The forwarder `environment` records on the cluster behind `rpc`, as the
/// EVM bindings return the ERC20 forwarder of an environment.
pub async fn spl_token_forwarder(
    rpc: &RpcClient,
    environment: Environment,
) -> Result<SplTokenForwarder<'_>, InstanceError> {
    Ok(SplTokenForwarder::at(
        rpc,
        recorded_program(rpc, environment).await?,
    ))
}

/// The forwarder `environment` records on the cluster behind `rpc`, read as
/// the emergency-migrating build, as the EVM bindings'
/// `emergency_migrating_erc20_forwarder` binds the draft.
pub async fn emergency_migrating_spl_token_forwarder(
    rpc: &RpcClient,
    environment: Environment,
) -> Result<EmergencyMigratingSplTokenForwarder<'_>, InstanceError> {
    Ok(EmergencyMigratingSplTokenForwarder::at(
        rpc,
        recorded_program(rpc, environment).await?,
    ))
}

/// The production build (`generated::spl_token_forwarder`).
pub enum Production {}

/// The emergency-migrating build
/// (`generated::emergency_migrating_spl_token_forwarder`).
pub enum EmergencyMigrating {}

/// The forwarder at an address, read over an RPC at its commitment, as the
/// build `B` it runs.
///
/// Its reads are exercised against a deployed forwarder by this repository's
/// integration tests (`crates/integration-test/tests/instance_reads.rs` and
/// `emergency_migrating.rs`).
pub struct Instance<'a, B> {
    rpc: &'a RpcClient,
    program_id: Pubkey,
    build: PhantomData<B>,
}

/// The production build at an address.
pub type SplTokenForwarder<'a> = Instance<'a, Production>;

/// The emergency-migrating build at an address.
pub type EmergencyMigratingSplTokenForwarder<'a> = Instance<'a, EmergencyMigrating>;

/// The reads and instructions both builds declare alike.
impl<'a, B> Instance<'a, B> {
    /// The forwarder at `program_id`, as alloy's `new(address, provider)`.
    pub fn at(rpc: &'a RpcClient, program_id: Pubkey) -> Self {
        Self {
            rpc,
            program_id,
            build: PhantomData,
        }
    }

    /// The address the forwarder is deployed at.
    pub fn program_id(&self) -> Pubkey {
        self.program_id
    }

    /// The forwarder's config, which both builds lay out alike.
    async fn config(&self) -> Result<Config, InstanceError> {
        let address = derive_forwarder_config_pda(&self.program_id).0;
        let what = format!("the forwarder's config {address}");
        let account = self
            .rpc
            .get_account_with_commitment(&address, self.rpc.commitment())
            .await
            .map_err(rpc_error(&what))?
            .value
            .ok_or(InstanceError::AccountMissing { address })?;
        Config::try_deserialize(&mut account.data.as_slice()).map_err(|e| InstanceError::Decode {
            what,
            error: e.to_string(),
        })
    }

    /// The data of the account at `address` when it exists and the forwarder
    /// owns it: an account the forwarder created.
    async fn owned_data(
        &self,
        address: Pubkey,
        what: &str,
    ) -> Result<Option<Vec<u8>>, InstanceError> {
        let account = self
            .rpc
            .get_account_with_commitment(&address, self.rpc.commitment())
            .await
            .map_err(rpc_error(&format!("{what} {address}")))?
            .value;
        Ok(account
            .filter(|account| account.owner == self.program_id)
            .map(|account| account.data))
    }

    /// `getLogicRef()`.
    pub async fn get_logic_ref(&self) -> Result<[u8; 32], InstanceError> {
        Ok(self.config().await?.logic_ref)
    }

    /// `getProtocolAdapter()`.
    pub async fn get_protocol_adapter(&self) -> Result<Pubkey, InstanceError> {
        Ok(self.config().await?.protocol_adapter)
    }

    /// `owner()`: the zero key once the ownership is renounced.
    pub async fn owner(&self) -> Result<Pubkey, InstanceError> {
        Ok(self.config().await?.owner)
    }

    /// `VERSION()`: the forwarder's `version` simulated, with `fee_payer`,
    /// any funded system account, paying the simulation's fee (nothing is
    /// signed or sent).
    pub async fn version(&self, fee_payer: &Pubkey) -> Result<String, InstanceError> {
        let message = Message::new(&[self.version_ix()], Some(fee_payer));
        let unsigned = VersionedTransaction {
            signatures: vec![
                Signature::default();
                usize::from(message.header.num_required_signatures)
            ],
            message: VersionedMessage::Legacy(message),
        };
        let simulated = self
            .rpc
            .simulate_transaction_with_config(
                &unsigned,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(self.rpc.commitment()),
                    ..RpcSimulateTransactionConfig::default()
                },
            )
            .await
            .map_err(rpc_error("the simulation of version"))?
            .value;
        if let Some(error) = simulated.err {
            return Err(InstanceError::Simulation {
                error: error.into(),
                logs: simulated.logs.unwrap_or_default(),
            });
        }
        returned::<String>(&self.program_id, simulated.return_data)
    }

    /// `initialize`, by `authority`, the program's upgrade authority.
    pub fn initialize_ix(
        &self,
        authority: &Pubkey,
        protocol_adapter: &Pubkey,
        logic_ref: [u8; 32],
        initial_owner: &Pubkey,
    ) -> Instruction {
        forwarder::initialize_ix(
            &self.program_id,
            authority,
            protocol_adapter,
            logic_ref,
            initial_owner,
        )
    }

    /// `init_nonce_bitmap` of `user`'s word `word_index`, `payer` funding
    /// the rent.
    pub fn init_nonce_bitmap_ix(
        &self,
        payer: &Pubkey,
        user: &Pubkey,
        word_index: u64,
    ) -> Instruction {
        forwarder::init_nonce_bitmap_ix(&self.program_id, payer, user, word_index)
    }

    /// `transfer_ownership` to `new_owner`, by the owner.
    pub fn transfer_ownership_ix(&self, owner: &Pubkey, new_owner: &Pubkey) -> Instruction {
        forwarder::transfer_ownership_ix(&self.program_id, owner, new_owner)
    }

    /// `renounce_ownership`, by the owner.
    pub fn renounce_ownership_ix(&self, owner: &Pubkey) -> Instruction {
        forwarder::renounce_ownership_ix(&self.program_id, owner)
    }

    /// `upgrade` to `buffer`'s code, by the owner, after the compute budget
    /// it needs.
    pub fn upgrade_ixs(&self, owner: &Pubkey, buffer: &Pubkey, spill: &Pubkey) -> [Instruction; 2] {
        forwarder::upgrade_ixs(&self.program_id, owner, buffer, spill)
    }

    /// `version`.
    pub fn version_ix(&self) -> Instruction {
        forwarder::version_ix(&self.program_id)
    }

    /// `close_escrow` of `mint`'s escrow into `recipient_ata`, by the owner,
    /// while the adapter whose state is `pa_state` is paused.
    pub fn close_escrow_ix(
        &self,
        owner: &Pubkey,
        pa_state: &Pubkey,
        mint: &Pubkey,
        recipient_ata: &Pubkey,
    ) -> Instruction {
        forwarder::close_escrow_ix(&self.program_id, owner, pa_state, mint, recipient_ata)
    }

    /// `close_nonce_bitmaps_batch` of `bitmaps`, by the owner, while the
    /// adapter whose state is `pa_state` is paused.
    pub fn close_nonce_bitmaps_batch_ix(
        &self,
        owner: &Pubkey,
        pa_state: &Pubkey,
        bitmaps: &[Pubkey],
    ) -> Instruction {
        forwarder::close_nonce_bitmaps_batch_ix(&self.program_id, owner, pa_state, bitmaps)
    }

    /// `close_config`, by the owner, while the adapter whose state is
    /// `pa_state` is paused.
    pub fn close_config_ix(&self, owner: &Pubkey, pa_state: &Pubkey) -> Instruction {
        forwarder::close_config_ix(&self.program_id, owner, pa_state)
    }
}

impl SplTokenForwarder<'_> {
    /// `reinitialize` to the resource logic `logic_ref`, by the owner.
    pub fn reinitialize_ix(&self, owner: &Pubkey, logic_ref: [u8; 32]) -> Instruction {
        forwarder::reinitialize_ix(&self.program_id, owner, logic_ref)
    }
}

impl EmergencyMigratingSplTokenForwarder<'_> {
    /// `isLogicRefVulnerable(logicRef)`: the forwarder lists `logic_ref`.
    /// False before `reinitialize` has listed any, as the EVM draft's empty
    /// storage answers.
    pub async fn is_logic_ref_vulnerable(
        &self,
        logic_ref: &[u8; 32],
    ) -> Result<bool, InstanceError> {
        let address = derive_vulnerable_logic_refs_pda(&self.program_id).0;
        let what = "the vulnerable logic refs";
        let Some(data) = self.owned_data(address, what).await? else {
            return Ok(false);
        };
        let listed = decode_vulnerable_logic_refs(&data).map_err(|e| InstanceError::Decode {
            what: format!("{what} {address}"),
            error: e.to_string(),
        })?;
        Ok(listed.contains(logic_ref))
    }

    /// `isNullifierMigrated(nullifier)`: the resource with `nullifier` has a
    /// migration record that says so. False without a record, as the EVM
    /// draft's empty storage answers.
    pub async fn is_nullifier_migrated(&self, nullifier: &[u8; 32]) -> Result<bool, InstanceError> {
        let address = derive_migration_record_pda(&self.program_id, nullifier).0;
        let what = "the migration record";
        let Some(data) = self.owned_data(address, what).await? else {
            return Ok(false);
        };
        let record = MigrationRecord::try_deserialize(&mut data.as_slice()).map_err(|e| {
            InstanceError::Decode {
                what: format!("{what} {address}"),
                error: e.to_string(),
            }
        })?;
        Ok(record.migrated)
    }

    /// `LEGACY_FORWARDER()`: the legacy forwarder whose resources migrate
    /// too, as the build's IDL records it.
    pub fn legacy_forwarder(&self) -> Pubkey {
        LEGACY_FORWARDER
    }

    /// `reinitialize`, by the owner: lists `vulnerable_logic_refs`, which the
    /// adapter `protocol_adapter` denies, and sets the logic ref to
    /// `new_logic_ref`.
    pub fn reinitialize_ix(
        &self,
        owner: &Pubkey,
        protocol_adapter: &Pubkey,
        new_logic_ref: [u8; 32],
        vulnerable_logic_refs: &[[u8; 32]],
    ) -> Instruction {
        emergency_migrating::reinitialize_ix(
            &self.program_id,
            owner,
            protocol_adapter,
            new_logic_ref,
            vulnerable_logic_refs,
        )
    }

    /// `init_migration_record` of the resource with `nullifier`, `payer`
    /// funding the rent.
    pub fn init_migration_record_ix(&self, payer: &Pubkey, nullifier: &[u8; 32]) -> Instruction {
        emergency_migrating::init_migration_record_ix(&self.program_id, payer, nullifier)
    }
}

/// The value of type `T` that `program` returned, as a simulation reports
/// it: all of the return data, nothing after it.
fn returned<T: AnchorDeserialize>(
    program: &Pubkey,
    return_data: Option<UiTransactionReturnData>,
) -> Result<T, InstanceError> {
    let what = format!("the return data of {program}");
    let decode = |error: String| InstanceError::Decode {
        what: what.clone(),
        error,
    };
    let data = return_data.ok_or_else(|| decode("the simulation returned nothing".to_owned()))?;
    if data.program_id != program.to_string() {
        return Err(decode(format!("{} returned it", data.program_id)));
    }
    let (encoded, UiReturnDataEncoding::Base64) = &data.data;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| decode(e.to_string()))?;
    T::try_from_slice(&bytes).map_err(|e| decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forwarder::tests::{assert_matches_the_idl, key};
    use solana_cluster_type::ClusterType;
    use std::collections::BTreeSet;

    fn genesis_hash(cluster: ClusterType) -> Hash {
        cluster.get_genesis_hash().unwrap()
    }

    #[test]
    fn the_recorded_staging_devnet_forwarder_is_not_deployed_yet() {
        let found = recorded_deployment(Environment::Staging, &genesis_hash(ClusterType::Devnet));
        println!("{found:?}");
        assert!(
            matches!(
                &found,
                Err(InstanceError::NoDeployment { chain_id })
                    if chain_id == "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1"
            ),
            "{found:?}"
        );
    }

    #[test]
    fn production_records_nothing_on_mainnet_beta() {
        let found = recorded_deployment(
            Environment::Production,
            &genesis_hash(ClusterType::MainnetBeta),
        );
        println!("{found:?}");
        assert!(
            matches!(
                &found,
                Err(InstanceError::NoDeployment { chain_id })
                    if chain_id == "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
            ),
            "{found:?}"
        );
    }

    /// A cluster no environment records, such as a local validator, is no
    /// deployment, under the chain id its genesis hash gives.
    #[test]
    fn an_unrecorded_genesis_hash_is_no_deployment_on_its_chain() {
        let unknown = Hash::new_from_array([0; 32]);
        for environment in [Environment::Staging, Environment::Production] {
            let found = recorded_deployment(environment, &unknown);
            println!("{environment:?}: {found:?}");
            assert!(
                matches!(
                    &found,
                    Err(InstanceError::NoDeployment { chain_id })
                        if chain_id == "solana:11111111111111111111111111111111"
                ),
                "{found:?}"
            );
        }
    }

    #[test]
    fn a_deployed_entry_is_the_clusters_deployment() {
        let record = Record::parse(
            r#"{"staging": [{"chainId": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
                             "programId": "34kd782wMwzkdwGFS5jfGGdTAz8zbaxVczQHs3E8oKF4",
                             "deploymentSlot": 4242}],
                "production": []}"#,
        );
        let found = deployment_in(
            &record,
            Environment::Staging,
            &genesis_hash(ClusterType::Devnet),
        );
        println!("{found:?}");
        assert_eq!(
            found.unwrap(),
            Deployment {
                chain_id: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".to_owned(),
                program_id: crate::generated::spl_token_forwarder::ID,
                deployment_slot: 4242,
            }
        );
        let other = deployment_in(
            &record,
            Environment::Production,
            &genesis_hash(ClusterType::Devnet),
        );
        assert!(
            matches!(other, Err(InstanceError::NoDeployment { .. })),
            "{other:?}"
        );
    }

    fn return_data(program: &Pubkey, bytes: &[u8]) -> Option<UiTransactionReturnData> {
        Some(UiTransactionReturnData {
            program_id: program.to_string(),
            data: (
                base64::engine::general_purpose::STANDARD.encode(bytes),
                UiReturnDataEncoding::Base64,
            ),
        })
    }

    #[test]
    fn the_returned_string_is_the_programs() {
        let program = key(9);
        // A Borsh String: its length as a little-endian u32, then the UTF-8.
        let encoded = [&5u32.to_le_bytes()[..], b"0.4.0"].concat();
        assert_eq!(
            returned::<String>(&program, return_data(&program, &encoded)).unwrap(),
            "0.4.0"
        );
        for (what, data) in [
            ("nothing", None),
            ("another program's", return_data(&key(8), &encoded)),
            (
                "a byte short",
                return_data(&program, &encoded[..encoded.len() - 1]),
            ),
            (
                "a trailing byte",
                return_data(&program, &[&encoded[..], &[0]].concat()),
            ),
            ("no bytes", return_data(&program, &[])),
            (
                "not UTF-8",
                return_data(&program, &[&1u32.to_le_bytes()[..], &[0xff]].concat()),
            ),
        ] {
            let result = returned::<String>(&program, data);
            println!("{what}: {result:?}");
            assert!(
                matches!(result, Err(InstanceError::Decode { .. })),
                "{what}: {result:?}"
            );
        }
    }

    /// `idl`'s instructions by name, but `forward_call`: only the adapter
    /// makes that call, inside a settlement, with the accounts the segment
    /// builders (`build_*_forwarder_accounts`) give it.
    fn sent_instructions(idl: &serde_json::Value) -> BTreeSet<String> {
        idl["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|ix| ix["name"].as_str().unwrap().to_owned())
            .filter(|name| name != "forward_call")
            .collect()
    }

    /// Each builder of the production build's instance is the module builder
    /// at the instance's address; there is one for every instruction a
    /// client sends, and each matches the build's IDL.
    #[test]
    fn the_production_builders_build_at_the_instances_address() {
        let rpc = RpcClient::new("http://127.0.0.1:1".to_owned());
        let p = key(9);
        let instance = SplTokenForwarder::at(&rpc, p);
        assert_eq!(instance.program_id(), p);
        let built = [
            (
                "initialize",
                instance.initialize_ix(&key(1), &key(2), [3; 32], &key(4)),
                forwarder::initialize_ix(&p, &key(1), &key(2), [3; 32], &key(4)),
            ),
            (
                "init_nonce_bitmap",
                instance.init_nonce_bitmap_ix(&key(1), &key(2), 7),
                forwarder::init_nonce_bitmap_ix(&p, &key(1), &key(2), 7),
            ),
            (
                "reinitialize",
                instance.reinitialize_ix(&key(1), [3; 32]),
                forwarder::reinitialize_ix(&p, &key(1), [3; 32]),
            ),
            (
                "transfer_ownership",
                instance.transfer_ownership_ix(&key(1), &key(2)),
                forwarder::transfer_ownership_ix(&p, &key(1), &key(2)),
            ),
            (
                "renounce_ownership",
                instance.renounce_ownership_ix(&key(1)),
                forwarder::renounce_ownership_ix(&p, &key(1)),
            ),
            (
                "upgrade",
                instance.upgrade_ixs(&key(1), &key(2), &key(3))[1].clone(),
                forwarder::upgrade_ixs(&p, &key(1), &key(2), &key(3))[1].clone(),
            ),
            ("version", instance.version_ix(), forwarder::version_ix(&p)),
            (
                "close_escrow",
                instance.close_escrow_ix(&key(1), &key(2), &key(3), &key(4)),
                forwarder::close_escrow_ix(&p, &key(1), &key(2), &key(3), &key(4)),
            ),
            (
                "close_nonce_bitmaps_batch",
                instance.close_nonce_bitmaps_batch_ix(&key(1), &key(2), &[key(5)]),
                forwarder::close_nonce_bitmaps_batch_ix(&p, &key(1), &key(2), &[key(5)]),
            ),
            (
                "close_config",
                instance.close_config_ix(&key(1), &key(2)),
                forwarder::close_config_ix(&p, &key(1), &key(2)),
            ),
        ];
        let idl = crate::idl::idl();
        let mut names = BTreeSet::new();
        for (name, built, expected) in built {
            println!("{name}: {} accounts", built.accounts.len());
            assert_eq!(built, expected, "{name}");
            assert!(names.insert(name.to_owned()), "{name} is built twice");
        }
        assert_eq!(
            names,
            sent_instructions(&idl),
            "the instance's builders and the IDL's instructions differ"
        );
        assert_eq!(
            instance.upgrade_ixs(&key(1), &key(2), &key(3)),
            forwarder::upgrade_ixs(&p, &key(1), &key(2), &key(3)),
            "upgrade, with its compute budget"
        );
    }

    /// The emergency-migrating build's instance builds every instruction a
    /// client sends that build, each the one its IDL declares: the builders
    /// both builds share as much as the build's own.
    #[test]
    fn the_emergency_migrating_builders_match_its_idl() {
        let rpc = RpcClient::new("http://127.0.0.1:1".to_owned());
        let p = crate::generated::emergency_migrating_spl_token_forwarder::ID;
        let instance = EmergencyMigratingSplTokenForwarder::at(&rpc, p);
        let (a, b, c, d) = (key(1), key(2), key(3), key(4));
        let refs = [[5; 32], [6; 32]];
        let built: [(&str, Instruction, Vec<u8>); 11] = [
            (
                "initialize",
                instance.initialize_ix(&a, &b, [3; 32], &d),
                [&b.to_bytes()[..], &[3; 32], &d.to_bytes()].concat(),
            ),
            (
                "init_nonce_bitmap",
                instance.init_nonce_bitmap_ix(&a, &b, 7),
                [&b.to_bytes()[..], &7u64.to_le_bytes()].concat(),
            ),
            (
                "init_migration_record",
                instance.init_migration_record_ix(&a, &[5; 32]),
                vec![5; 32],
            ),
            (
                "reinitialize",
                instance.reinitialize_ix(&a, &b, [3; 32], &refs),
                [&[3; 32][..], &2u32.to_le_bytes(), &[5; 32], &[6; 32]].concat(),
            ),
            (
                "transfer_ownership",
                instance.transfer_ownership_ix(&a, &b),
                b.to_bytes().to_vec(),
            ),
            (
                "renounce_ownership",
                instance.renounce_ownership_ix(&a),
                vec![],
            ),
            (
                "upgrade",
                instance.upgrade_ixs(&a, &b, &c)[1].clone(),
                vec![],
            ),
            ("version", instance.version_ix(), vec![]),
            (
                "close_escrow",
                instance.close_escrow_ix(&a, &b, &c, &d),
                vec![],
            ),
            (
                "close_nonce_bitmaps_batch",
                instance.close_nonce_bitmaps_batch_ix(&a, &b, &[key(5)]),
                vec![],
            ),
            ("close_config", instance.close_config_ix(&a, &b), vec![]),
        ];
        let idl = crate::idl::emergency_migrating_idl();
        let mut names = BTreeSet::new();
        for (name, ix, args) in built {
            println!("{name}: {} accounts", ix.accounts.len());
            assert_eq!(ix.program_id, p, "{name}");
            let _ = assert_matches_the_idl(&idl, &ix, name, &args);
            assert!(names.insert(name.to_owned()), "{name} is built twice");
        }
        assert_eq!(
            names,
            sent_instructions(&idl),
            "the instance's builders and the IDL's instructions differ"
        );
    }

    /// The reads both builds share decode one `Config`: the builds declare it
    /// alike.
    #[test]
    fn both_builds_declare_one_config() {
        let config = |idl: &serde_json::Value| {
            let declared = |list: &str| {
                idl[list]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|item| item["name"] == "Config")
                    .cloned()
                    .unwrap_or_else(|| panic!("the IDL's {list} have no Config"))
            };
            (declared("accounts"), declared("types"))
        };
        let production = config(&crate::idl::idl());
        let emergency_migrating = config(&crate::idl::emergency_migrating_idl());
        println!("{production:?}");
        assert_eq!(production, emergency_migrating);
    }

    /// `legacy_forwarder` is the emergency-migrating IDL's `LEGACY_FORWARDER`.
    #[test]
    fn the_legacy_forwarder_is_the_idls() {
        let idl = crate::idl::emergency_migrating_idl();
        let declared = idl["constants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|constant| constant["name"] == "LEGACY_FORWARDER")
            .expect("the IDL declares LEGACY_FORWARDER")["value"]
            .as_str()
            .unwrap()
            .to_owned();
        let rpc = RpcClient::new("http://127.0.0.1:1".to_owned());
        let instance = EmergencyMigratingSplTokenForwarder::at(&rpc, key(9));
        println!("LEGACY_FORWARDER: {declared}");
        assert_eq!(instance.legacy_forwarder().to_string(), declared);
    }
}
