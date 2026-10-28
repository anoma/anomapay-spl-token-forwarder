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

/// The env file `cluster`'s deployment is built and operated with
/// (`env/<cluster>.env`).
fn env_file(cluster: ClusterType) -> &'static str {
    match cluster {
        ClusterType::Devnet => include_str!("../../../env/devnet.env"),
        other => panic!("no env file is named for {other:?}; name env/<cluster>.env here"),
    }
}

/// The value `env_file` sets `variable` to, which it sets once.
fn env_value<'a>(env_file: &'a str, variable: &str) -> &'a str {
    let prefix = format!("{variable}=");
    let values: Vec<&str> = env_file
        .lines()
        .filter_map(|line| line.strip_prefix(prefix.as_str()))
        .collect();
    assert_eq!(
        values.len(),
        1,
        "the env file sets {variable} {} times: {values:?}",
        values.len()
    );
    values[0]
}

/// The operator scripts deploy and operate the devnet forwarder at
/// `env/devnet.env`'s `FORWARDER_PROGRAM_ID`: the record names the same one.
#[test]
fn the_records_devnet_address_is_env_devnets() {
    let devnet = staging_devnet_entry();
    let address = env_value(env_file(ClusterType::Devnet), "FORWARDER_PROGRAM_ID");
    println!(
        "staging devnet: {}, env/devnet.env: {address}",
        devnet["programId"]
    );
    assert_eq!(devnet["programId"].as_str(), Some(address));
}

/// The emergency-migrating build a cluster runs takes its legacy forwarder
/// from the cluster's env file, and its instance answers
/// `legacy_forwarder` from the IDL's `LEGACY_FORWARDER`: each recorded
/// cluster's env file names the IDL's, and the record's program is the env
/// file's.
#[test]
fn every_recorded_clusters_env_file_names_the_idls_legacy_forwarder_and_program() {
    let idl: serde_json::Value = serde_json::from_str(include_str!(
        "../idls/emergency_migrating_spl_token_forwarder.json"
    ))
    .unwrap();
    let declared = idl["constants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|constant| constant["name"] == "LEGACY_FORWARDER")
        .expect("the emergency-migrating IDL declares LEGACY_FORWARDER")["value"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut checked = 0;
    for environment in ENVIRONMENTS {
        for entry in raw_entries(environment) {
            let cluster = cluster_of(&entry);
            let env = env_file(cluster);
            let legacy = env_value(env, "LEGACY_FORWARDER_PROGRAM_ID");
            let program = env_value(env, "FORWARDER_PROGRAM_ID");
            println!(
                "{environment:?} {cluster:?}: LEGACY_FORWARDER_PROGRAM_ID {legacy}, the IDL's \
                 LEGACY_FORWARDER {declared}; FORWARDER_PROGRAM_ID {program}, recorded {}",
                entry["programId"]
            );
            assert_eq!(
                legacy, declared,
                "{environment:?} {cluster:?}: env/<cluster>.env's LEGACY_FORWARDER_PROGRAM_ID is \
                 {legacy}, the emergency-migrating IDL's LEGACY_FORWARDER is {declared}"
            );
            assert_eq!(
                entry["programId"].as_str(),
                Some(program),
                "{environment:?} {cluster:?}: the record's program is {}, the env file's \
                 FORWARDER_PROGRAM_ID is {program}",
                entry["programId"]
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the record has no entry to check");
}
