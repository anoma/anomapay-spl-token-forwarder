//! The forwarder deployed at an address, as the EVM bindings return the
//! ERC20 forwarder (`contract::erc20_forwarder`): its reads under their EVM
//! names, from its config and the emergency-migrating build's accounts, and
//! its instructions, built for its address. One type serves each build
//! ([`SplTokenForwarderInstance`],
//! [`EmergencyMigratingSplTokenForwarderInstance`]); the reads and
//! instructions both builds declare are written once, and each build adds its
//! own.

use std::marker::PhantomData;

use anchor_lang::{AccountDeserialize, AnchorDeserialize};
use base64::Engine;
use solana_instruction::Instruction;
use solana_message::{Hash, Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_rpc_client_api::response::{UiReturnDataEncoding, UiTransactionReturnData};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use crate::accounts::decode_vulnerable_logic_refs;
use crate::addresses::{clusters, spl_token_forwarder_address, Environment};
use crate::emergency_migrating::{
    self, derive_migration_record_pda, derive_vulnerable_logic_refs_pda,
};
use crate::error::{rpc_transport_error, BindingsError, BindingsResult};
use crate::forwarder;
use crate::generated::emergency_migrating_spl_token_forwarder::accounts::MigrationRecord;
use crate::generated::emergency_migrating_spl_token_forwarder::constants::LEGACY_FORWARDER;
use crate::generated::spl_token_forwarder::accounts::Config;
use crate::pda::derive_forwarder_config_pda;

/// Returns an SPL token forwarder instance of the environment for the given
/// RPC client. An emergency-migrating build at the recorded address is
/// `EmergencyMigratingSplTokenForwarderInstance::new` of the instance's
/// `address()`.
pub async fn spl_token_forwarder(
    rpc: &RpcClient,
    environment: Environment,
) -> BindingsResult<SplTokenForwarderInstance<'_>> {
    let genesis_hash = rpc
        .get_genesis_hash()
        .await
        .map_err(rpc_transport_error("the genesis hash"))?;
    let address = recorded_address(environment, &genesis_hash)?;
    Ok(SplTokenForwarderInstance::new(address, rpc))
}

/// The forwarder `environment` records on the cluster whose genesis hash is
/// `genesis_hash`.
fn recorded_address(environment: Environment, genesis_hash: &Hash) -> BindingsResult<Pubkey> {
    let cluster = clusters()
        .find(|cluster| cluster.get_genesis_hash().as_ref() == Some(genesis_hash))
        .ok_or(BindingsError::ChainIdUnknown)?;
    spl_token_forwarder_address(environment, &cluster)
        .ok_or(BindingsError::UnsupportedChain(cluster))
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
/// integration tests (`crates/integration-test/tests/contract_reads.rs` and
/// `emergency_migrating.rs`).
pub struct Instance<'a, B> {
    address: Pubkey,
    rpc: &'a RpcClient,
    build: PhantomData<B>,
}

/// The production build at an address.
pub type SplTokenForwarderInstance<'a> = Instance<'a, Production>;

/// The emergency-migrating build at an address.
pub type EmergencyMigratingSplTokenForwarderInstance<'a> = Instance<'a, EmergencyMigrating>;

/// The reads and instructions both builds declare alike.
impl<'a, B> Instance<'a, B> {
    /// The forwarder at `address`, as alloy's
    /// `ERC20ForwarderInstance::new(address, provider)`.
    pub fn new(address: Pubkey, rpc: &'a RpcClient) -> Self {
        Self {
            address,
            rpc,
            build: PhantomData,
        }
    }

    /// The address the forwarder is deployed at.
    pub fn address(&self) -> Pubkey {
        self.address
    }

    /// The forwarder's config, which both builds lay out alike.
    async fn config(&self) -> BindingsResult<Config> {
        let address = derive_forwarder_config_pda(&self.address).0;
        let what = format!("the forwarder's config {address}");
        let account = self
            .rpc
            .get_account_with_commitment(&address, self.rpc.commitment())
            .await
            .map_err(rpc_transport_error(&what))?
            .value
            .ok_or(BindingsError::AccountMissing { address })?;
        Config::try_deserialize(&mut account.data.as_slice()).map_err(|e| BindingsError::Decode {
            what,
            error: e.to_string(),
        })
    }

    /// The data of the account at `address` when it exists and the forwarder
    /// owns it: an account the forwarder created.
    async fn owned_data(&self, address: Pubkey, what: &str) -> BindingsResult<Option<Vec<u8>>> {
        let account = self
            .rpc
            .get_account_with_commitment(&address, self.rpc.commitment())
            .await
            .map_err(rpc_transport_error(&format!("{what} {address}")))?
            .value;
        Ok(account
            .filter(|account| account.owner == self.address)
            .map(|account| account.data))
    }

    /// `getLogicRef()`.
    pub async fn get_logic_ref(&self) -> BindingsResult<[u8; 32]> {
        Ok(self.config().await?.logic_ref)
    }

    /// `getProtocolAdapter()`.
    pub async fn get_protocol_adapter(&self) -> BindingsResult<Pubkey> {
        Ok(self.config().await?.protocol_adapter)
    }

    /// `owner()`: the zero key once the ownership is renounced.
    pub async fn owner(&self) -> BindingsResult<Pubkey> {
        Ok(self.config().await?.owner)
    }

    /// `VERSION()`: the forwarder's `version` simulated, with `fee_payer`,
    /// any funded system account, paying the simulation's fee (nothing is
    /// signed or sent).
    pub async fn version(&self, fee_payer: &Pubkey) -> BindingsResult<String> {
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
            .map_err(rpc_transport_error("the simulation of version"))?
            .value;
        if let Some(error) = simulated.err {
            return Err(BindingsError::Simulation {
                error: error.into(),
                logs: simulated.logs.unwrap_or_default(),
            });
        }
        returned::<String>(&self.address, simulated.return_data)
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
            &self.address,
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
        forwarder::init_nonce_bitmap_ix(&self.address, payer, user, word_index)
    }

    /// `transfer_ownership` to `new_owner`, by the owner.
    pub fn transfer_ownership_ix(&self, owner: &Pubkey, new_owner: &Pubkey) -> Instruction {
        forwarder::transfer_ownership_ix(&self.address, owner, new_owner)
    }

    /// `renounce_ownership`, by the owner.
    pub fn renounce_ownership_ix(&self, owner: &Pubkey) -> Instruction {
        forwarder::renounce_ownership_ix(&self.address, owner)
    }

    /// `upgrade` to `buffer`'s code, by the owner, after the compute budget
    /// it needs.
    pub fn upgrade_ixs(&self, owner: &Pubkey, buffer: &Pubkey, spill: &Pubkey) -> [Instruction; 2] {
        forwarder::upgrade_ixs(&self.address, owner, buffer, spill)
    }

    /// `version`.
    pub fn version_ix(&self) -> Instruction {
        forwarder::version_ix(&self.address)
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
        forwarder::close_escrow_ix(&self.address, owner, pa_state, mint, recipient_ata)
    }

    /// `close_nonce_bitmaps_batch` of `bitmaps`, by the owner, while the
    /// adapter whose state is `pa_state` is paused.
    pub fn close_nonce_bitmaps_batch_ix(
        &self,
        owner: &Pubkey,
        pa_state: &Pubkey,
        bitmaps: &[Pubkey],
    ) -> Instruction {
        forwarder::close_nonce_bitmaps_batch_ix(&self.address, owner, pa_state, bitmaps)
    }

    /// `close_config`, by the owner, while the adapter whose state is
    /// `pa_state` is paused.
    pub fn close_config_ix(&self, owner: &Pubkey, pa_state: &Pubkey) -> Instruction {
        forwarder::close_config_ix(&self.address, owner, pa_state)
    }
}

impl SplTokenForwarderInstance<'_> {
    /// `reinitialize` to the resource logic `logic_ref`, by the owner.
    pub fn reinitialize_ix(&self, owner: &Pubkey, logic_ref: [u8; 32]) -> Instruction {
        forwarder::reinitialize_ix(&self.address, owner, logic_ref)
    }
}

impl EmergencyMigratingSplTokenForwarderInstance<'_> {
    /// `isLogicRefVulnerable(logicRef)`: the forwarder lists `logic_ref`.
    /// False before `reinitialize` has listed any, as the EVM draft's empty
    /// storage answers.
    pub async fn is_logic_ref_vulnerable(&self, logic_ref: &[u8; 32]) -> BindingsResult<bool> {
        let address = derive_vulnerable_logic_refs_pda(&self.address).0;
        let what = "the vulnerable logic refs";
        let Some(data) = self.owned_data(address, what).await? else {
            return Ok(false);
        };
        let listed = decode_vulnerable_logic_refs(&data).map_err(|e| BindingsError::Decode {
            what: format!("{what} {address}"),
            error: e.to_string(),
        })?;
        Ok(listed.contains(logic_ref))
    }

    /// `isNullifierMigrated(nullifier)`: the resource with `nullifier` has a
    /// migration record that says so. False without a record, as the EVM
    /// draft's empty storage answers.
    pub async fn is_nullifier_migrated(&self, nullifier: &[u8; 32]) -> BindingsResult<bool> {
        let address = derive_migration_record_pda(&self.address, nullifier).0;
        let what = "the migration record";
        let Some(data) = self.owned_data(address, what).await? else {
            return Ok(false);
        };
        let record = MigrationRecord::try_deserialize(&mut data.as_slice()).map_err(|e| {
            BindingsError::Decode {
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
            &self.address,
            owner,
            protocol_adapter,
            new_logic_ref,
            vulnerable_logic_refs,
        )
    }

    /// `init_migration_record` of the resource with `nullifier`, `payer`
    /// funding the rent.
    pub fn init_migration_record_ix(&self, payer: &Pubkey, nullifier: &[u8; 32]) -> Instruction {
        emergency_migrating::init_migration_record_ix(&self.address, payer, nullifier)
    }
}

/// The value of type `T` that `program` returned, as a simulation reports
/// it: all of the return data, nothing after it.
fn returned<T: AnchorDeserialize>(
    program: &Pubkey,
    return_data: Option<UiTransactionReturnData>,
) -> BindingsResult<T> {
    let what = format!("the return data of {program}");
    let decode = |error: String| BindingsError::Decode {
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
    fn the_recorded_staging_devnet_forwarder_is_the_idls_program() {
        let found = recorded_address(Environment::Staging, &genesis_hash(ClusterType::Devnet));
        println!("{found:?}");
        assert!(
            matches!(found, Ok(address) if address == crate::generated::spl_token_forwarder::ID),
            "{found:?}"
        );
    }

    #[test]
    fn production_records_nothing_on_mainnet_beta() {
        let found = recorded_address(
            Environment::Production,
            &genesis_hash(ClusterType::MainnetBeta),
        );
        println!("{found:?}");
        assert!(
            matches!(
                found,
                Err(BindingsError::UnsupportedChain(ClusterType::MainnetBeta))
            ),
            "{found:?}"
        );
    }

    /// A genesis hash of no known cluster, such as a local validator's, is
    /// `ChainIdUnknown` in every environment.
    #[test]
    fn a_genesis_hash_of_no_known_cluster_is_chain_id_unknown() {
        let unknown = Hash::new_from_array([0; 32]);
        for environment in [Environment::Staging, Environment::Production] {
            let found = recorded_address(environment, &unknown);
            println!("{environment:?}: {found:?}");
            assert!(
                matches!(found, Err(BindingsError::ChainIdUnknown)),
                "{found:?}"
            );
        }
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
                matches!(result, Err(BindingsError::Decode { .. })),
                "{what}: {result:?}"
            );
        }
    }

    /// Requires `result`, a read over an RPC nothing answers, to fail with
    /// `RpcTransportError` naming what the read fetches its answer from.
    fn assert_transport_error<T: std::fmt::Debug>(
        read: &str,
        what: &str,
        result: BindingsResult<T>,
    ) {
        println!("{read}: {result:?}");
        assert!(
            matches!(&result, Err(BindingsError::RpcTransportError { what: fetched, .. }) if fetched.starts_with(what)),
            "{read} is {result:?}, not RpcTransportError fetching {what}"
        );
    }

    /// Nothing listens on port 1: each read fails, naming the account or
    /// simulation it reads, so each read asks for what it answers from.
    #[tokio::test]
    async fn every_read_fetches_what_it_answers_from() {
        let rpc = RpcClient::new("http://127.0.0.1:1".to_owned());
        let production = SplTokenForwarderInstance::new(key(9), &rpc);
        let emergency = EmergencyMigratingSplTokenForwarderInstance::new(key(9), &rpc);
        let config = "the forwarder's config";
        let version = "the simulation of version";
        assert_transport_error("get_logic_ref", config, production.get_logic_ref().await);
        assert_transport_error(
            "get_protocol_adapter",
            config,
            production.get_protocol_adapter().await,
        );
        assert_transport_error("owner", config, production.owner().await);
        assert_transport_error("version", version, production.version(&key(1)).await);
        assert_transport_error("get_logic_ref", config, emergency.get_logic_ref().await);
        assert_transport_error(
            "get_protocol_adapter",
            config,
            emergency.get_protocol_adapter().await,
        );
        assert_transport_error("owner", config, emergency.owner().await);
        assert_transport_error("version", version, emergency.version(&key(1)).await);
        assert_transport_error(
            "is_logic_ref_vulnerable",
            "the vulnerable logic refs",
            emergency.is_logic_ref_vulnerable(&[1; 32]).await,
        );
        assert_transport_error(
            "is_nullifier_migrated",
            "the migration record",
            emergency.is_nullifier_migrated(&[1; 32]).await,
        );
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
        let instance = SplTokenForwarderInstance::new(p, &rpc);
        assert_eq!(instance.address(), p);
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
        let instance = EmergencyMigratingSplTokenForwarderInstance::new(p, &rpc);
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
        let instance = EmergencyMigratingSplTokenForwarderInstance::new(key(9), &rpc);
        println!("LEGACY_FORWARDER: {declared}");
        assert_eq!(instance.legacy_forwarder().to_string(), declared);
    }
}
