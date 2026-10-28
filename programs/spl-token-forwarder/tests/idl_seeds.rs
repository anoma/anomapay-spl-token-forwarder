//! Every `arg` seed in the forwarder's committed IDLs, and in the IDL of the
//! adapter it builds against, names one of its instruction's arguments, so
//! Anchor's TypeScript account resolver can fill its account.

use std::path::PathBuf;
use std::process::Command;

use anchor_lang::idl::types::Idl;
use protocol_adapter::idl_seeds::arg_seeds;

fn parsed(name: &str, json: &str) -> Idl {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{name} is not an Anchor IDL: {e}"))
}

/// The IDL the adapter commits beside the program crate this forwarder
/// builds against, `idl/protocol_adapter.json` of the locked revision,
/// located through `cargo metadata`.
fn adapter_idl() -> (PathBuf, String) {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()
        .expect("failed to run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata printed no JSON");
    let manifest = metadata["packages"]
        .as_array()
        .expect("cargo metadata lists no packages")
        .iter()
        .find(|package| package["name"] == "protocol-adapter")
        .expect("the forwarder depends on no protocol-adapter")["manifest_path"]
        .as_str()
        .expect("protocol-adapter has no manifest path");
    // programs/solana-pa-prototype/Cargo.toml → idl/protocol_adapter.json
    let path = PathBuf::from(manifest)
        .ancestors()
        .nth(3)
        .expect("the adapter's manifest is under its workspace")
        .join("idl/protocol_adapter.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    (path, json)
}

#[test]
fn every_arg_seed_names_an_instruction_arg() {
    let (adapter_path, adapter_json) = adapter_idl();
    println!("the adapter's IDL: {}", adapter_path.display());
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
        ("protocol_adapter.json", adapter_json.as_str()),
    ] {
        let seeds = arg_seeds(&parsed(name, json));
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
