//! Every `arg` seed in the forwarder's committed IDLs names one of its
//! instruction's arguments, so Anchor's TypeScript account resolver can fill
//! its account.

use anchor_lang::idl::types::Idl;
use protocol_adapter::idl_seeds::arg_seeds;

#[test]
fn every_arg_seed_names_an_instruction_arg() {
    let mut failures = Vec::new();
    for (name, json) in [
        (
            "spl_token_forwarder.json",
            include_str!("../../../crates/client/idls/spl_token_forwarder.json"),
        ),
        (
            "emergency_migrating_spl_token_forwarder.json",
            include_str!(
                "../../../crates/client/idls/emergency_migrating_spl_token_forwarder.json"
            ),
        ),
    ] {
        let idl: Idl = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("{name} is not an Anchor IDL: {e}"));
        let seeds = arg_seeds(&idl);
        for seed in &seeds {
            println!("{name}: {seed:?}");
        }
        assert!(!seeds.is_empty(), "{name} has no arg seeds to check");
        failures.extend(
            seeds
                .into_iter()
                .filter(|seed| !seed.names_an_arg())
                .map(|seed| (name, seed)),
        );
    }
    assert!(
        failures.is_empty(),
        "arg seeds naming no instruction arg: {failures:#?}"
    );
}
