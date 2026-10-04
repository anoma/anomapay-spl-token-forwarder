//! The repository's operator scripts (`scripts/*.ts`), run against an
//! environment's runtime as `ops.sh` runs them against a cluster: through
//! ts-node, with the endpoint, the wallet and the adapter's address in the
//! environment. They need `yarn install` and the production build's IDL and
//! types (`ops.sh script-test` prepares both).

use std::path::PathBuf;
use std::process::Output;

use anoma_pa_solana_integration_test::envs::common::environment::Environment;
use anyhow::Context;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// The repository root, where ops.sh runs the scripts.
fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// What a script printed and how it exited.
pub struct Ran {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl std::fmt::Display for Ran {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit {}\nstdout:\n{}\nstderr:\n{}",
            if self.success { "0" } else { "non-zero" },
            self.stdout,
            self.stderr
        )
    }
}

/// Runs `scripts/<script>` with `args` against `env`'s runtime, signing with
/// `wallet`, with `vars` set besides the endpoint, the wallet and the
/// adapter's address.
pub fn run_script<P>(
    env: &Environment<P>,
    wallet: &Keypair,
    script: &str,
    args: &[&str],
    vars: &[(&str, String)],
) -> anyhow::Result<Ran> {
    let wallet_file = std::env::temp_dir().join(format!(
        "operator-script-wallet-{}-{}.json",
        std::process::id(),
        wallet.pubkey()
    ));
    std::fs::write(
        &wallet_file,
        serde_json::to_string(&wallet.to_bytes().to_vec())?,
    )
    .with_context(|| format!("failed to write {}", wallet_file.display()))?;
    let output: std::io::Result<Output> = std::process::Command::new("npx")
        .args(["ts-node", "-P", "tsconfig.json"])
        .arg(format!("scripts/{script}"))
        .args(args)
        .current_dir(repository())
        .env("ANCHOR_PROVIDER_URL", env.surfnet.rpc_url())
        .env("ANCHOR_WS_URL", env.surfnet.ws_url())
        .env("ANCHOR_WALLET", &wallet_file)
        .env(
            "PROTOCOL_ADAPTER_PROGRAM_ID",
            env.protocol_adapter.program.to_string(),
        )
        .envs(vars.iter().map(|(name, value)| (*name, value.as_str())))
        .output();
    std::fs::remove_file(&wallet_file)
        .with_context(|| format!("failed to remove {}", wallet_file.display()))?;
    let output = output.with_context(|| format!("failed to run scripts/{script}"))?;
    Ok(Ran {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// `bytes` as lowercase hex, as the scripts read a logic ref.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
