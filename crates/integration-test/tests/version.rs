//! The forwarder answers `version` with the release it is, as the EVM
//! forwarder exposes `VERSION`: read from the deployed program, it names the
//! code an address runs, which an in-place upgrade changes. The release is
//! the crate version, which the IDL also records.

use anomapay_spl_token_forwarder_client::version_ix;
use anomapay_spl_token_forwarder_integration_test::setup;
use base64::Engine;
use solana_signer::Signer;
use solana_transaction::Transaction;

#[tokio::test(flavor = "multi_thread")]
async fn the_forwarder_answers_with_the_release_its_idl_records() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let rpc = &env.protocol_adapter.rpc;
    let payer = &env.protocol_adapter.payer;
    let transaction = Transaction::new_signed_with_payer(
        &[version_ix(&local.forwarder.program)],
        Some(&payer.pubkey()),
        &[payer.as_ref()],
        rpc.get_latest_blockhash().await?,
    );
    let simulated = rpc.simulate_transaction(&transaction).await?.value;
    anyhow::ensure!(
        simulated.err.is_none(),
        "version fails: {:?} {:?}",
        simulated.err,
        simulated.logs
    );
    let returned = simulated
        .return_data
        .ok_or_else(|| anyhow::anyhow!("version returns nothing"))?;
    let data = base64::engine::general_purpose::STANDARD.decode(&returned.data.0)?;
    // A Borsh String: its length as a little-endian u32, then the UTF-8.
    let (len, text) = data
        .split_first_chunk::<4>()
        .ok_or_else(|| anyhow::anyhow!("version returns {data:?}, not a string"))?;
    anyhow::ensure!(
        u32::from_le_bytes(*len) as usize == text.len(),
        "version returns {data:?}, not one string"
    );
    let version = std::str::from_utf8(text)?;

    let idl: serde_json::Value =
        serde_json::from_str(include_str!("../../client/idl/spl_token_forwarder.json"))?;
    anyhow::ensure!(
        idl["metadata"]["version"] == version,
        "the deployed program answers {version}, the IDL records {}",
        idl["metadata"]["version"]
    );
    Ok(())
}
