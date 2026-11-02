//! The deployment record against the env files the operator scripts deploy
//! and operate each cluster's forwarder with. They need no cluster and hold
//! everywhere, so they run on every push.

use anomapay_spl_token_forwarder_client::addresses::{
    spl_token_forwarder_deployments_map, Environment,
};
use solana_cluster_type::ClusterType;

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
    for environment in [Environment::Staging, Environment::Production] {
        for (cluster, recorded) in spl_token_forwarder_deployments_map(environment) {
            let env = env_file(cluster);
            let legacy = env_value(env, "LEGACY_FORWARDER_PROGRAM_ID");
            let program = env_value(env, "FORWARDER_PROGRAM_ID");
            println!(
                "{environment:?} {cluster:?}: LEGACY_FORWARDER_PROGRAM_ID {legacy}, the IDL's \
                 LEGACY_FORWARDER {declared}; FORWARDER_PROGRAM_ID {program}, recorded {recorded}"
            );
            assert_eq!(
                legacy, declared,
                "{environment:?} {cluster:?}: env/<cluster>.env's LEGACY_FORWARDER_PROGRAM_ID is \
                 {legacy}, the emergency-migrating IDL's LEGACY_FORWARDER is {declared}"
            );
            assert_eq!(
                recorded.to_string(),
                program,
                "{environment:?} {cluster:?}: the record's program is {recorded}, the env file's \
                 FORWARDER_PROGRAM_ID is {program}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the record has no deployment to check");
}
