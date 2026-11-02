//! The forwarder's deployments, as `deployments.json` records them in the
//! adapter client's deployment-record format (`DeploymentRecord`): per
//! environment, the program on each cluster, under the cluster's CAIP-2
//! chain id. The environments are the adapter client's, so one value names an
//! environment to both clients.

use anoma_pa_solana_client::addresses::DeploymentRecord;
pub use anoma_pa_solana_client::addresses::Environment;
use solana_cluster_type::ClusterType;
use solana_pubkey::Pubkey;
use std::sync::LazyLock;

static DEPLOYMENTS: LazyLock<DeploymentRecord> =
    LazyLock::new(|| DeploymentRecord::parse(include_str!("../deployments.json")));

/// Returns a map of the SPL token forwarder programs recorded for the
/// environment.
pub fn spl_token_forwarder_deployments_map(environment: Environment) -> Vec<(ClusterType, Pubkey)> {
    DEPLOYMENTS.of(environment).to_vec()
}

/// Returns the SPL token forwarder program recorded for the environment on
/// the provided cluster, if any. Both builds run at the one address: the
/// emergency-migrating build replaces the production build in place.
pub fn spl_token_forwarder_address(
    environment: Environment,
    cluster: &ClusterType,
) -> Option<Pubkey> {
    DEPLOYMENTS.address(environment, cluster)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client is generated from the IDLs of the program staging runs:
    /// both builds declare its address.
    #[test]
    fn the_staging_devnet_program_is_the_vendored_idls_address() {
        let devnet = spl_token_forwarder_address(Environment::Staging, &ClusterType::Devnet);
        for (name, id) in [
            (
                "spl_token_forwarder",
                crate::generated::spl_token_forwarder::ID,
            ),
            (
                "emergency_migrating_spl_token_forwarder",
                crate::generated::emergency_migrating_spl_token_forwarder::ID,
            ),
        ] {
            println!("staging devnet: {devnet:?}, {name} IDL: {id}");
            assert_eq!(devnet, Some(id), "{name}");
        }
    }

    /// `deployments.json` is a record as the adapter client's parser reads
    /// it: a misspelled, missing or extra field, or an entry the parser
    /// refuses, fails here.
    #[test]
    fn the_committed_record_parses() {
        println!("deployments.json: {:?}", *DEPLOYMENTS);
    }
}
