//! The closing instructions, which reclaim the rent of the forwarder's
//! accounts: the owner's, refused while the adapter runs, and then closing
//! the nonce bitmaps, the escrows and, last, the config.

use anoma_pa_solana_client::{anchor_account_disc, derive_pa_state_pda};
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anoma_pa_testkit::environment::{ProtocolAdapter as _, Refusal};
use anoma_pa_testkit::execute_tx;
use anomapay_spl_token_forwarder_client::{
    close_config_ix, close_escrow_ix, close_nonce_bitmaps_batch_ix, derive_forwarder_config_pda,
    derive_forwarder_escrow_authority, derive_nonce_bitmap_pda, init_nonce_bitmap_ix,
};
use anomapay_spl_token_forwarder_integration_test::fixtures::ShieldedOwner;
use anomapay_spl_token_forwarder_integration_test::refusal::{NOT_THE_OWNER, balances};
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, OwnedForwarder, balance, create_mint, fund, mint_to, token_account,
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_rpc_client_types::config::RpcProgramAccountsConfig;
use solana_rpc_client_types::filter::{Memcmp, RpcFilterType};
use solana_signer::Signer;
use spl_associated_token_account_interface::address::get_associated_token_address;
use surfpool_sdk::Pubkey;

/// What a test escrows of a fresh mint: 500 tokens at 6 decimals.
const ESCROWED: u64 = 500_000_000;

fn pa_state(env: &LocalEnv) -> Pubkey {
    derive_pa_state_pda(&env.protocol_adapter.program).0
}

/// Sending `ix` signed by `signers` fails with `error`.
async fn refuses(
    env: &LocalEnv,
    ix: Instruction,
    signers: &[&Keypair],
    error: &'static str,
) -> anyhow::Result<()> {
    expect_integration_panic(Needle::Static(error))(env.send(&[ix], signers).await)
}

/// The nonce bitmaps the forwarder `program` holds.
async fn nonce_bitmaps(env: &LocalEnv, program: &Pubkey) -> anyhow::Result<Vec<Pubkey>> {
    let accounts = env
        .protocol_adapter
        .rpc
        .get_program_ui_accounts_with_config(
            program,
            RpcProgramAccountsConfig {
                filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                    0,
                    anchor_account_disc("NonceBitmap").to_vec(),
                ))]),
                ..RpcProgramAccountsConfig::default()
            },
        )
        .await?;
    Ok(accounts.into_iter().map(|(address, _)| address).collect())
}

/// An escrow of a fresh mint holding `ESCROWED`, and a token account of the
/// mint owned by `recipient`: the mint, the escrow account and the
/// recipient's.
async fn funded_escrow(
    env: &LocalEnv,
    local: &OwnedForwarder,
    recipient: &Pubkey,
) -> anyhow::Result<(Pubkey, Pubkey, Pubkey)> {
    let program = local.forwarder.program;
    let mint = create_mint(env, &program).await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let escrow = get_associated_token_address(&escrow_authority, &mint);
    mint_to(env, &mint, &escrow, ESCROWED).await?;
    let recipient_account = token_account(env, recipient, &mint).await?;
    Ok((mint, escrow, recipient_account))
}

/// The local environment with the adapter paused by its owner.
async fn paused() -> anyhow::Result<(LocalEnv, OwnedForwarder)> {
    let (mut env, local) = setup::local().await?;
    env.protocol_adapter.pause().await?;
    Ok((env, local))
}

// Closing while the adapter runs would drain the escrow, forget used nonces
// or disable the forwarder under live resources.

#[tokio::test(flavor = "multi_thread")]
async fn refuses_close_escrow_while_the_adapter_is_running() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let owner = local.owner.pubkey();
    let (mint, escrow, recipient_account) = funded_escrow(&env, &local, &owner).await?;
    let ix = close_escrow_ix(
        &local.forwarder.program,
        &owner,
        &pa_state(&env),
        &mint,
        &recipient_account,
    );
    refuses(
        &env,
        ix,
        &[&local.owner],
        "Error Code: ProtocolAdapterNotPaused.",
    )
    .await?;
    anyhow::ensure!(
        balances(&env, &[escrow, recipient_account]).await? == [ESCROWED, 0],
        "no tokens move"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_close_nonce_bitmaps_batch_while_the_adapter_is_running() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let program = local.forwarder.program;
    let user = Keypair::new().pubkey();
    env.send(
        &[init_nonce_bitmap_ix(
            &program,
            &env.protocol_adapter.payer.pubkey(),
            &user,
            0,
        )],
        &[],
    )
    .await?;
    let bitmaps = nonce_bitmaps(&env, &program).await?;
    anyhow::ensure!(!bitmaps.is_empty(), "the forwarder holds a nonce bitmap");
    let ix =
        close_nonce_bitmaps_batch_ix(&program, &local.owner.pubkey(), &pa_state(&env), &bitmaps);
    refuses(
        &env,
        ix,
        &[&local.owner],
        "Error Code: ProtocolAdapterNotPaused.",
    )
    .await?;
    anyhow::ensure!(
        nonce_bitmaps(&env, &program).await? == bitmaps,
        "the bitmaps are not closed"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_close_config_while_the_adapter_is_running() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let ix = close_config_ix(
        &local.forwarder.program,
        &local.owner.pubkey(),
        &pa_state(&env),
    );
    refuses(
        &env,
        ix,
        &[&local.owner],
        "Error Code: ProtocolAdapterNotPaused.",
    )
    .await
}

// Only the owner closes, as only the owner can replace the program's code.

#[tokio::test(flavor = "multi_thread")]
async fn close_escrow_refuses_anyone_but_the_owner() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let stranger = Keypair::new();
    let (mint, _, recipient_account) = funded_escrow(&env, &local, &stranger.pubkey()).await?;
    let ix = close_escrow_ix(
        &local.forwarder.program,
        &stranger.pubkey(),
        &pa_state(&env),
        &mint,
        &recipient_account,
    );
    refuses(&env, ix, &[&stranger], NOT_THE_OWNER).await
}

#[tokio::test(flavor = "multi_thread")]
async fn close_nonce_bitmaps_batch_refuses_anyone_but_the_owner() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let stranger = Keypair::new();
    let ix = close_nonce_bitmaps_batch_ix(
        &local.forwarder.program,
        &stranger.pubkey(),
        &pa_state(&env),
        &[],
    );
    refuses(&env, ix, &[&stranger], NOT_THE_OWNER).await
}

#[tokio::test(flavor = "multi_thread")]
async fn close_config_refuses_anyone_but_the_owner() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let stranger = Keypair::new();
    let ix = close_config_ix(
        &local.forwarder.program,
        &stranger.pubkey(),
        &pa_state(&env),
    );
    refuses(&env, ix, &[&stranger], NOT_THE_OWNER).await
}

// The config is the forwarder's but is not a nonce bitmap.
#[tokio::test(flavor = "multi_thread")]
async fn close_nonce_bitmaps_batch_refuses_a_program_account_that_is_not_a_bitmap()
-> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let program = local.forwarder.program;
    let ix = close_nonce_bitmaps_batch_ix(
        &program,
        &local.owner.pubkey(),
        &pa_state(&env),
        &[derive_forwarder_config_pda(&program).0],
    );
    refuses(
        &env,
        ix,
        &[&local.owner],
        "Error Code: InvalidNonceBitmapPda.",
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn closes_every_nonce_bitmap_and_refunds_their_rent() -> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    let program = local.forwarder.program;
    let payer = env.protocol_adapter.payer.pubkey();
    let users = [Keypair::new().pubkey(), Keypair::new().pubkey()];
    env.send(
        &users.map(|user| init_nonce_bitmap_ix(&program, &payer, &user, 0)),
        &[],
    )
    .await?;
    env.protocol_adapter.pause().await?;
    let bitmaps = nonce_bitmaps(&env, &program).await?;
    anyhow::ensure!(
        bitmaps.len() == users.len()
            && users
                .iter()
                .all(|user| bitmaps.contains(&derive_nonce_bitmap_pda(&program, user, 0).0)),
        "the forwarder holds the bitmaps {bitmaps:?}, not the two created"
    );

    let owner = local.owner.pubkey();
    let rpc = &env.protocol_adapter.rpc;
    let before = rpc.get_balance(&owner).await?;
    env.send(
        &[close_nonce_bitmaps_batch_ix(
            &program,
            &owner,
            &pa_state(&env),
            &bitmaps,
        )],
        &[&local.owner],
    )
    .await?;
    anyhow::ensure!(
        nonce_bitmaps(&env, &program).await?.is_empty(),
        "every nonce bitmap is closed"
    );
    anyhow::ensure!(
        rpc.get_balance(&owner).await? > before,
        "the owner recovers the bitmaps' rent"
    );
    Ok(())
}

// Closing the bitmaps forgets which nonces were used, and the adapter's
// owner can unpause the adapter afterwards. The user's signature on a
// settled wrap then passes the forwarder's nonce check again; resubmitting
// that wrap must still fail, on the adapter's record of its spent
// nullifiers, with no tokens moved.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_settled_wrap_replayed_after_its_nonce_bitmap_is_closed() -> anyhow::Result<()> {
    const AMOUNT: u64 = 100_000_000;
    const NONCE: u64 = 1;
    let (mut env, local) = setup::local().await?;
    let forwarder = &local.forwarder;
    let program = forwarder.program;
    let wrap = forwarder
        .prove_wrap(
            &env,
            &ShieldedOwner::seeded("teardown/replay-owner"),
            AMOUNT,
            NONCE,
            "teardown/replay",
        )
        .await?;
    execute_tx(&mut env, wrap.tx.clone()).await?;

    env.protocol_adapter.pause().await?;
    let bitmaps = nonce_bitmaps(&env, &program).await?;
    env.send(
        &[close_nonce_bitmaps_batch_ix(
            &program,
            &local.owner.pubkey(),
            &pa_state(&env),
            &bitmaps,
        )],
        &[&local.owner],
    )
    .await?;
    anyhow::ensure!(
        nonce_bitmaps(&env, &program).await?.is_empty(),
        "every nonce bitmap is closed, so the forwarder no longer records the wrap's nonce"
    );
    env.protocol_adapter.unpause().await?;

    // The user holds and approves the amount again, so a replay that got
    // through would move it.
    fund(&env, &program, &forwarder.mint, &forwarder.user, AMOUNT).await?;
    let accounts = [forwarder.user_account(), forwarder.escrow_account()];
    let before = balances(&env, &accounts).await?;
    let refusal = env.protocol_adapter.submit(wrap.tx).await?.err();
    anyhow::ensure!(
        refusal == Some(Refusal::NullifierSpent),
        "the adapter returned {refusal:?} for the replayed wrap, not NullifierSpent"
    );
    anyhow::ensure!(
        balances(&env, &accounts).await? == before,
        "the replayed wrap moved tokens"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn close_escrow_drains_the_tokens_to_the_recipient_and_closes_the_account()
-> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let owner = local.owner.pubkey();
    let (mint, escrow, recipient_account) = funded_escrow(&env, &local, &owner).await?;
    let rpc = &env.protocol_adapter.rpc;
    let before = rpc.get_balance(&owner).await?;
    env.send(
        &[close_escrow_ix(
            &local.forwarder.program,
            &owner,
            &pa_state(&env),
            &mint,
            &recipient_account,
        )],
        &[&local.owner],
    )
    .await?;
    anyhow::ensure!(
        rpc.get_account_with_commitment(&escrow, rpc.commitment())
            .await?
            .value
            .is_none(),
        "the escrow account is closed"
    );
    anyhow::ensure!(
        balance(&env, &recipient_account).await? == ESCROWED,
        "every escrowed token is drained to the recipient"
    );
    anyhow::ensure!(
        rpc.get_balance(&owner).await? > before,
        "the escrow account's rent returns to the owner"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn close_escrow_closes_an_empty_escrow() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let program = local.forwarder.program;
    let owner = local.owner.pubkey();
    let mint = create_mint(&env, &program).await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let escrow = get_associated_token_address(&escrow_authority, &mint);
    let recipient_account = token_account(&env, &owner, &mint).await?;
    anyhow::ensure!(balance(&env, &escrow).await? == 0, "the escrow is empty");
    env.send(
        &[close_escrow_ix(
            &program,
            &owner,
            &pa_state(&env),
            &mint,
            &recipient_account,
        )],
        &[&local.owner],
    )
    .await?;
    let rpc = &env.protocol_adapter.rpc;
    anyhow::ensure!(
        rpc.get_account_with_commitment(&escrow, rpc.commitment())
            .await?
            .value
            .is_none(),
        "the empty escrow account is closed"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn close_config_closes_the_config_and_refunds_its_rent() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let program = local.forwarder.program;
    let owner = local.owner.pubkey();
    let rpc = &env.protocol_adapter.rpc;
    let before = rpc.get_balance(&owner).await?;
    env.send(
        &[close_config_ix(&program, &owner, &pa_state(&env))],
        &[&local.owner],
    )
    .await?;
    let (config, _) = derive_forwarder_config_pda(&program);
    anyhow::ensure!(
        rpc.get_account_with_commitment(&config, rpc.commitment())
            .await?
            .value
            .is_none(),
        "the config is closed"
    );
    anyhow::ensure!(
        rpc.get_balance(&owner).await? > before,
        "the config's rent returns to the owner"
    );
    Ok(())
}
