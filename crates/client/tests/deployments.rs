//! Checks of the deployment record itself. They need no cluster and hold
//! everywhere, so they run on every push.

mod common;

use anomapay_spl_token_forwarder_client::generated::{
    emergency_migrating_spl_token_forwarder, spl_token_forwarder,
};
use common::{cluster_of, raw_entries, ENVIRONMENTS};
use solana_cluster_type::ClusterType;
use solana_pubkey::Pubkey;
use std::collections::HashSet;

#[test]
fn every_chain_id_is_a_known_clusters() {
    for environment in ENVIRONMENTS {
        for entry in raw_entries(environment) {
            let cluster = cluster_of(&entry);
            println!("{environment:?}: {} is {cluster:?}", entry["chainId"]);
        }
    }
}

#[test]
fn no_chain_id_repeats_within_an_environment() {
    for environment in ENVIRONMENTS {
        let mut seen = HashSet::new();
        for entry in raw_entries(environment) {
            let id = entry["chainId"].to_string();
            assert!(
                seen.insert(id.clone()),
                "{environment:?} records {id} twice"
            );
        }
    }
}

#[test]
fn every_program_id_parses() {
    for environment in ENVIRONMENTS {
        for entry in raw_entries(environment) {
            let id = entry["programId"]
                .as_str()
                .unwrap_or_else(|| panic!("{environment:?}: programId is no string: {entry}"));
            let program: Pubkey = id
                .parse()
                .unwrap_or_else(|e| panic!("{environment:?}: program id {id}: {e}"));
            println!("{environment:?}: {program}");
        }
    }
}

/// Staging's one devnet entry, as written.
fn staging_devnet_entry() -> serde_json::Value {
    let entries = raw_entries(common::Environment::Staging);
    let devnet: Vec<_> = entries
        .into_iter()
        .filter(|entry| cluster_of(entry) == ClusterType::Devnet)
        .collect();
    assert_eq!(devnet.len(), 1, "staging's devnet entries: {devnet:?}");
    devnet.into_iter().next().unwrap()
}

/// The client is generated from the IDLs of the program staging runs: both
/// builds declare its address.
#[test]
fn the_staging_devnet_program_is_the_vendored_idls_address() {
    let devnet = staging_devnet_entry();
    for (name, idl, id) in [
        (
            "spl_token_forwarder",
            include_str!("../idls/spl_token_forwarder.json"),
            spl_token_forwarder::ID,
        ),
        (
            "emergency_migrating_spl_token_forwarder",
            include_str!("../idls/emergency_migrating_spl_token_forwarder.json"),
            emergency_migrating_spl_token_forwarder::ID,
        ),
    ] {
        let idl: serde_json::Value = serde_json::from_str(idl).unwrap();
        println!(
            "staging devnet: {}, {name} IDL: {}",
            devnet["programId"], idl["address"]
        );
        assert_eq!(devnet["programId"], idl["address"], "{name}");
        assert_eq!(
            devnet["programId"].as_str().unwrap(),
            id.to_string(),
            "{name}"
        );
    }
}

/// The operator scripts deploy and operate the devnet forwarder at
/// `env/devnet.env`'s `FORWARDER_PROGRAM_ID`: the record names the same one.
#[test]
fn the_records_devnet_address_is_env_devnets() {
    let devnet = staging_devnet_entry();
    let env_file = include_str!("../../../env/devnet.env");
    let lines: Vec<&str> = env_file
        .lines()
        .filter_map(|line| line.strip_prefix("FORWARDER_PROGRAM_ID="))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "env/devnet.env sets FORWARDER_PROGRAM_ID {} times",
        lines.len()
    );
    println!(
        "staging devnet: {}, env/devnet.env: {}",
        devnet["programId"], lines[0]
    );
    assert_eq!(devnet["programId"].as_str(), Some(lines[0]));
}
