//! The forwarder's deployments, as `deployments.json` records them: per
//! environment, the program on each cluster, under the cluster's CAIP-2 chain
//! id. The environments and the chain-id rule are the adapter client's
//! (`anoma_pa_solana_client::deployments`), so both records name a cluster
//! alike.

use anoma_pa_solana_client::deployments::Environment;
use serde::Deserialize;
use solana_pubkey::Pubkey;
use std::sync::LazyLock;

/// A recorded forwarder deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deployment {
    /// The cluster's CAIP-2 chain id
    /// (`anoma_pa_solana_client::deployments::chain_id`).
    pub chain_id: String,
    /// The forwarder program.
    pub program_id: Pubkey,
    /// The slot of the program's first deployment, where its history starts.
    pub deployment_slot: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    chain_id: String,
    program_id: String,
    deployment_slot: Option<u64>,
}

/// A deployment record, as `deployments.json` holds one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    staging: Vec<Entry>,
    production: Vec<Entry>,
}

impl Record {
    /// The record `json` holds, or a panic saying why it is none.
    pub(crate) fn parse(json: &str) -> Self {
        serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("not a deployment record: {e}: {json}"))
    }

    /// The forwarder `environment` records on the cluster `chain_id` names,
    /// or `None` when it records none there or records it as not yet
    /// deployed (a `null` deployment slot).
    pub(crate) fn deployment(
        &self,
        environment: Environment,
        chain_id: &str,
    ) -> Option<Deployment> {
        let entries = match environment {
            Environment::Staging => &self.staging,
            Environment::Production => &self.production,
        };
        let entry = entries.iter().find(|entry| entry.chain_id == chain_id)?;
        Some(Deployment {
            chain_id: entry.chain_id.clone(),
            program_id: entry.program_id.parse().unwrap_or_else(|e| {
                panic!(
                    "deployment record: {environment:?} program id {} on {chain_id}: {e}",
                    entry.program_id
                )
            }),
            deployment_slot: entry.deployment_slot?,
        })
    }
}

/// `deployments.json`.
pub(crate) static RECORD: LazyLock<Record> =
    LazyLock::new(|| Record::parse(include_str!("../deployments.json")));

/// The forwarder `environment` records on the cluster `chain_id` names, or
/// `None` when it records none there or records it as not yet deployed (a
/// `null` deployment slot).
pub fn deployment(environment: Environment, chain_id: &str) -> Option<Deployment> {
    RECORD.deployment(environment, chain_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record of one entry in each environment: `staging_slot` the
    /// devnet entry's slot, mainnet-beta's 9.
    fn record(staging_slot: &str) -> Record {
        Record::parse(&format!(
            r#"{{"staging": [{{"chainId": "solana:a", "programId": "34kd782wMwzkdwGFS5jfGGdTAz8zbaxVczQHs3E8oKF4", "deploymentSlot": {staging_slot}}}],
                "production": [{{"chainId": "solana:b", "programId": "11111111111111111111111111111111", "deploymentSlot": 9}}]}}"#
        ))
    }

    #[test]
    fn the_entry_on_the_chain_is_its_deployment() {
        let record = record("7");
        let found = record.deployment(Environment::Production, "solana:b");
        println!("{found:?}");
        assert_eq!(
            found,
            Some(Deployment {
                chain_id: "solana:b".to_string(),
                program_id: Pubkey::default(),
                deployment_slot: 9,
            })
        );
        assert_eq!(
            record
                .deployment(Environment::Staging, "solana:a")
                .map(|d| d.deployment_slot),
            Some(7)
        );
        assert_eq!(record.deployment(Environment::Staging, "solana:b"), None);
        assert_eq!(record.deployment(Environment::Production, "solana:c"), None);
    }

    /// `deployments.json` is a record as the parser reads it: a misspelled,
    /// missing or extra field fails here.
    #[test]
    fn the_committed_record_parses() {
        let record = &*RECORD;
        println!(
            "deployments.json: {} staging, {} production entries",
            record.staging.len(),
            record.production.len()
        );
    }

    /// An entry whose program is not deployed yet has a `null` slot.
    #[test]
    fn an_entry_without_a_deployment_slot_is_no_deployment() {
        assert_eq!(
            record("null").deployment(Environment::Staging, "solana:a"),
            None
        );
    }
}
