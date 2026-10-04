//! The emergency committee's instructions: refused while the adapter runs;
//! once it is paused, naming the emergency caller, who withdraws from
//! escrow, and the teardown that reclaims the forwarder's rent.

use anoma_pa_solana_client::{anchor_account_disc, derive_pa_state_pda, pause_ix};
use anoma_pa_solana_integration_test::envs::local::Environment as LocalEnv;
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anomapay_spl_token_forwarder_client::{
    UnwrapInput, close_config_ix, close_escrow_ix, close_nonce_bitmaps_batch_ix, decode_config,
    derive_associated_token_address, derive_forwarder_config_pda,
    derive_forwarder_escrow_authority, derive_nonce_bitmap_pda, forward_emergency_call_ix,
    init_nonce_bitmap_ix, set_emergency_caller_ix,
};
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup::{
    self, LocalForwarder, balance, create_mint, fund, mint_to, token_account,
};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_rpc_client_types::config::RpcProgramAccountsConfig;
use solana_rpc_client_types::filter::{Memcmp, RpcFilterType};
use solana_signer::Signer;
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
    local: &LocalForwarder,
    recipient: &Pubkey,
) -> anyhow::Result<(Pubkey, Pubkey, Pubkey)> {
    let program = local.forwarder.program;
    let mint = create_mint(env, &program).await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let escrow = derive_associated_token_address(&escrow_authority, &mint);
    mint_to(env, &mint, &escrow, ESCROWED).await?;
    let recipient_account = token_account(env, recipient, &mint).await?;
    Ok((mint, escrow, recipient_account))
}

/// The local environment with the adapter paused by its owner.
async fn paused() -> anyhow::Result<(LocalEnv, LocalForwarder)> {
    let (env, local) = setup::local().await?;
    env.send(
        &[pause_ix(
            &env.protocol_adapter.program,
            &env.protocol_adapter.payer.pubkey(),
        )],
        &[],
    )
    .await?;
    Ok((env, local))
}

// Mirrors EmergencyMigratableForwarderBase: the committee acts only once the
// adapter is paused. Teardown while running would drain the escrow, forget
// used nonces or disable the forwarder under live resources.

// Mirrors EmergencyMigratableForwarderBase.t.sol: test_setEmergencyCaller_reverts_if_the_pa_is_not_stopped
#[tokio::test(flavor = "multi_thread")]
async fn refuses_set_emergency_caller_while_the_adapter_is_running() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let ix = set_emergency_caller_ix(
        &local.forwarder.program,
        &local.committee.pubkey(),
        &pa_state(&env),
        &Keypair::new().pubkey(),
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
        "Error Code: ProtocolAdapterNotPaused.",
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_close_escrow_while_the_adapter_is_running() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let recipient = local.committee.pubkey();
    let (mint, escrow, recipient_account) = funded_escrow(&env, &local, &recipient).await?;
    let ix = close_escrow_ix(
        &local.forwarder.program,
        &recipient,
        &pa_state(&env),
        &mint,
        &recipient_account,
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
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
    let ix = close_nonce_bitmaps_batch_ix(
        &program,
        &local.committee.pubkey(),
        &pa_state(&env),
        &bitmaps,
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
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
        &local.committee.pubkey(),
        &pa_state(&env),
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
        "Error Code: ProtocolAdapterNotPaused.",
    )
    .await
}

/// The emergency caller's withdrawal of `amount` of `mint` to `recipient`'s
/// token account, signed by `caller`.
fn withdraw(
    env: &LocalEnv,
    local: &LocalForwarder,
    caller: &Pubkey,
    mint: &Pubkey,
    recipient: &Pubkey,
    amount: u64,
) -> Instruction {
    forward_emergency_call_ix(
        &local.forwarder.program,
        caller,
        &pa_state(env),
        &UnwrapInput {
            token_mint: mint.to_bytes(),
            amount,
            recipient: recipient.to_bytes(),
        },
    )
}

/// Where `forward_emergency_call_ix` passes the escrow's token account and
/// the recipient's.
const EMERGENCY_SOURCE: usize = 5;
const EMERGENCY_DESTINATION: usize = 6;

/// The committee names `caller` the emergency caller.
async fn set_emergency_caller(
    env: &LocalEnv,
    local: &LocalForwarder,
    caller: &Pubkey,
) -> anyhow::Result<()> {
    env.send(
        &[set_emergency_caller_ix(
            &local.forwarder.program,
            &local.committee.pubkey(),
            &pa_state(env),
            caller,
        )],
        &[&local.committee],
    )
    .await?;
    Ok(())
}

// Mirrors: test_forwardEmergencyCall_reverts_if_the_pa_is_stopped_but_the_emergency_caller_is_not_set
#[tokio::test(flavor = "multi_thread")]
async fn refuses_forward_emergency_call_before_an_emergency_caller_is_set() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let (recipient, caller) = (Keypair::new().pubkey(), Keypair::new());
    let (mint, _, _) = funded_escrow(&env, &local, &recipient).await?;
    let ix = withdraw(&env, &local, &caller.pubkey(), &mint, &recipient, 1000);
    refuses(&env, ix, &[&caller], "Error Code: EmergencyCallerNotSet.").await
}

// Mirrors: test_setEmergencyCaller_reverts_if_the_new_emergency_caller_is_the_zero_address
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_zero_emergency_caller() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let ix = set_emergency_caller_ix(
        &local.forwarder.program,
        &local.committee.pubkey(),
        &pa_state(&env),
        &Pubkey::default(),
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
        "Error Code: ZeroAddressNotAllowed.",
    )
    .await
}

// Mirrors: test_setEmergencyCaller_sets_the_emergency_caller and
// test_emergencyCaller_returns_the_emergency_caller_after_it_has_been_set
#[tokio::test(flavor = "multi_thread")]
async fn the_committee_sets_the_emergency_caller_once_the_adapter_is_paused() -> anyhow::Result<()>
{
    let (env, local) = paused().await?;
    let caller = Keypair::new().pubkey();
    set_emergency_caller(&env, &local, &caller).await?;
    let (config, _) = derive_forwarder_config_pda(&local.forwarder.program);
    let config = decode_config(&env.protocol_adapter.rpc.get_account_data(&config).await?)?;
    anyhow::ensure!(
        config.emergency_caller == caller.to_bytes(),
        "the config names {} the emergency caller, not {caller}",
        Pubkey::new_from_array(config.emergency_caller)
    );
    Ok(())
}

// Mirrors: test_setEmergencyCaller_reverts_if_the_emergency_caller_has_already_been_set
#[tokio::test(flavor = "multi_thread")]
async fn refuses_setting_the_emergency_caller_twice() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    set_emergency_caller(&env, &local, &Keypair::new().pubkey()).await?;
    let ix = set_emergency_caller_ix(
        &local.forwarder.program,
        &local.committee.pubkey(),
        &pa_state(&env),
        &Keypair::new().pubkey(),
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
        "Error Code: EmergencyCallerAlreadySet.",
    )
    .await
}

/// The paused environment with an emergency caller named, a funded escrow
/// of a fresh mint, and a recipient with a token account of it: the
/// caller, the mint, the escrow account, the recipient and its account.
async fn emergency() -> anyhow::Result<(
    LocalEnv,
    LocalForwarder,
    Keypair,
    Pubkey,
    Pubkey,
    Pubkey,
    Pubkey,
)> {
    let (env, local) = paused().await?;
    let caller = Keypair::new();
    set_emergency_caller(&env, &local, &caller.pubkey()).await?;
    let recipient = Keypair::new().pubkey();
    let (mint, escrow, recipient_account) = funded_escrow(&env, &local, &recipient).await?;
    Ok((
        env,
        local,
        caller,
        mint,
        escrow,
        recipient,
        recipient_account,
    ))
}

// Mirrors: test_forwardEmergencyCall_reverts_if_the_pa_is_stopped_but_the_caller_is_not_the_emergency_caller
#[tokio::test(flavor = "multi_thread")]
async fn refuses_forward_emergency_call_from_anyone_but_the_emergency_caller() -> anyhow::Result<()>
{
    let (env, local, _, mint, _, recipient, _) = emergency().await?;
    let wrong = Keypair::new();
    let ix = withdraw(&env, &local, &wrong.pubkey(), &mint, &recipient, 1000);
    refuses(&env, ix, &[&wrong], "Error Code: UnauthorizedCaller.").await
}

// The destination is chosen by the caller, not by the input; it must be the
// recipient's.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_withdrawal_to_a_token_account_the_recipient_does_not_own() -> anyhow::Result<()>
{
    let (env, local, caller, mint, escrow, recipient, _) = emergency().await?;
    let foreign = token_account(&env, &Keypair::new().pubkey(), &mint).await?;
    let mut ix = withdraw(&env, &local, &caller.pubkey(), &mint, &recipient, 1000);
    ix.accounts[EMERGENCY_DESTINATION].pubkey = foreign;
    refuses(&env, ix, &[&caller], "Error Code: WrongTokenAccountOwner.").await?;
    anyhow::ensure!(
        balances(&env, &[escrow, foreign]).await? == [ESCROWED, 0],
        "no tokens move"
    );
    Ok(())
}

// The escrow authority signs the withdrawal; it pays only from an account
// the escrow owns.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_withdrawal_whose_source_the_escrow_does_not_own() -> anyhow::Result<()> {
    let (env, local, caller, mint, _, recipient, recipient_account) = emergency().await?;
    let other = fund(&env, &local.forwarder.program, &mint, &Keypair::new(), 1000).await?;
    let mut ix = withdraw(&env, &local, &caller.pubkey(), &mint, &recipient, 1000);
    ix.accounts[EMERGENCY_SOURCE].pubkey = other;
    refuses(&env, ix, &[&caller], "Error Code: WrongTokenAccountOwner.").await?;
    anyhow::ensure!(
        balances(&env, &[other, recipient_account]).await? == [1000, 0],
        "no tokens move"
    );
    Ok(())
}

// Mirrors: test_forwardEmergencyCall_forwards_calls_if_the_pa_is_stopped_and_the_caller_is_the_emergency_caller
#[tokio::test(flavor = "multi_thread")]
async fn the_emergency_caller_withdraws_from_escrow() -> anyhow::Result<()> {
    let (env, local, caller, mint, escrow, recipient, recipient_account) = emergency().await?;
    let amount = 25_000_000;
    env.send(
        &[withdraw(
            &env,
            &local,
            &caller.pubkey(),
            &mint,
            &recipient,
            amount,
        )],
        &[&caller],
    )
    .await?;
    anyhow::ensure!(
        balances(&env, &[escrow, recipient_account]).await? == [ESCROWED - amount, amount],
        "the withdrawal moves {amount} from escrow to the recipient"
    );
    Ok(())
}

// The committee reclaims the rent of the nonce bitmaps, the escrows and,
// last, the config.

#[tokio::test(flavor = "multi_thread")]
async fn close_escrow_refuses_a_non_committee_authority() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let impostor = Keypair::new();
    let (mint, _, recipient_account) = funded_escrow(&env, &local, &impostor.pubkey()).await?;
    let ix = close_escrow_ix(
        &local.forwarder.program,
        &impostor.pubkey(),
        &pa_state(&env),
        &mint,
        &recipient_account,
    );
    refuses(&env, ix, &[&impostor], "Error Code: UnauthorizedCaller.").await
}

#[tokio::test(flavor = "multi_thread")]
async fn close_nonce_bitmaps_batch_refuses_a_non_committee_authority() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let impostor = Keypair::new();
    let ix = close_nonce_bitmaps_batch_ix(
        &local.forwarder.program,
        &impostor.pubkey(),
        &pa_state(&env),
        &[],
    );
    refuses(&env, ix, &[&impostor], "Error Code: UnauthorizedCaller.").await
}

#[tokio::test(flavor = "multi_thread")]
async fn close_config_refuses_a_non_committee_authority() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let impostor = Keypair::new();
    let ix = close_config_ix(
        &local.forwarder.program,
        &impostor.pubkey(),
        &pa_state(&env),
    );
    refuses(&env, ix, &[&impostor], "Error Code: UnauthorizedCaller.").await
}

// The config is the forwarder's but is not a nonce bitmap.
#[tokio::test(flavor = "multi_thread")]
async fn close_nonce_bitmaps_batch_refuses_a_program_account_that_is_not_a_bitmap()
-> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let program = local.forwarder.program;
    let ix = close_nonce_bitmaps_batch_ix(
        &program,
        &local.committee.pubkey(),
        &pa_state(&env),
        &[derive_forwarder_config_pda(&program).0],
    );
    refuses(
        &env,
        ix,
        &[&local.committee],
        "Error Code: InvalidNonceBitmapPda.",
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn closes_every_nonce_bitmap_and_refunds_their_rent() -> anyhow::Result<()> {
    let (env, local) = setup::local().await?;
    let program = local.forwarder.program;
    let payer = env.protocol_adapter.payer.pubkey();
    let users = [Keypair::new().pubkey(), Keypair::new().pubkey()];
    env.send(
        &users.map(|user| init_nonce_bitmap_ix(&program, &payer, &user, 0)),
        &[],
    )
    .await?;
    env.send(&[pause_ix(&env.protocol_adapter.program, &payer)], &[])
        .await?;
    let bitmaps = nonce_bitmaps(&env, &program).await?;
    anyhow::ensure!(
        bitmaps.len() == users.len()
            && users
                .iter()
                .all(|user| bitmaps.contains(&derive_nonce_bitmap_pda(&program, user, 0).0)),
        "the forwarder holds the bitmaps {bitmaps:?}, not the two created"
    );

    let committee = local.committee.pubkey();
    let rpc = &env.protocol_adapter.rpc;
    let before = rpc.get_balance(&committee).await?;
    env.send(
        &[close_nonce_bitmaps_batch_ix(
            &program,
            &committee,
            &pa_state(&env),
            &bitmaps,
        )],
        &[&local.committee],
    )
    .await?;
    anyhow::ensure!(
        nonce_bitmaps(&env, &program).await?.is_empty(),
        "every nonce bitmap is closed"
    );
    anyhow::ensure!(
        rpc.get_balance(&committee).await? > before,
        "the committee recovers the bitmaps' rent"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn close_escrow_drains_the_tokens_to_the_recipient_and_closes_the_account()
-> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let committee = local.committee.pubkey();
    let (mint, escrow, recipient_account) = funded_escrow(&env, &local, &committee).await?;
    let rpc = &env.protocol_adapter.rpc;
    let before = rpc.get_balance(&committee).await?;
    env.send(
        &[close_escrow_ix(
            &local.forwarder.program,
            &committee,
            &pa_state(&env),
            &mint,
            &recipient_account,
        )],
        &[&local.committee],
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
        rpc.get_balance(&committee).await? > before,
        "the escrow account's rent returns to the committee"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn close_escrow_closes_an_empty_escrow() -> anyhow::Result<()> {
    let (env, local) = paused().await?;
    let program = local.forwarder.program;
    let committee = local.committee.pubkey();
    let mint = create_mint(&env, &program).await?;
    let (escrow_authority, _) = derive_forwarder_escrow_authority(&program);
    let escrow = derive_associated_token_address(&escrow_authority, &mint);
    let recipient_account = token_account(&env, &committee, &mint).await?;
    anyhow::ensure!(balance(&env, &escrow).await? == 0, "the escrow is empty");
    env.send(
        &[close_escrow_ix(
            &program,
            &committee,
            &pa_state(&env),
            &mint,
            &recipient_account,
        )],
        &[&local.committee],
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
    let committee = local.committee.pubkey();
    let rpc = &env.protocol_adapter.rpc;
    let before = rpc.get_balance(&committee).await?;
    env.send(
        &[close_config_ix(&program, &committee, &pa_state(&env))],
        &[&local.committee],
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
        rpc.get_balance(&committee).await? > before,
        "the config's rent returns to the committee"
    );
    Ok(())
}
