//! Only the adapter itself may call the forwarder: a program the adapter
//! invokes during a settlement (the adapter's test forwarder, in relay mode)
//! that calls the forwarder in turn is refused, and no tokens move.

use std::sync::Arc;

use anoma_pa_solana_client::external_call::{OutputMode, SolanaExternalCall};
use anoma_pa_solana_integration_test::forwarders::{CallAccounts, Forwarder};
use anoma_pa_solana_integration_test::test_forwarder::{RELAY_OK, relay_input};
use anoma_pa_testkit::assert::{Needle, expect_integration_panic};
use anoma_pa_testkit::fixtures::passthrough;
use anoma_pa_testkit::witness::{AppData, ExpirableBlob};
use anoma_pa_testkit::{execute_tx, prove_actions};
use anoma_rm_risc0::utils::bytes_to_words;
use anomapay_spl_token_forwarder_client::{
    FORWARDER_UNWRAP_NUM_ACCOUNTS, build_unwrap_forwarder_accounts, encode_unwrap_forwarder_input,
};
use anomapay_spl_token_forwarder_integration_test::logic::logic_ref;
use anomapay_spl_token_forwarder_integration_test::refusal::balances;
use anomapay_spl_token_forwarder_integration_test::setup::{self, mint_to, token_account};
use futures::future::BoxFuture;
use solana_instruction::AccountMeta;
use solana_keypair::Keypair;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use surfpool_sdk::Pubkey;

/// How a submitter passes the test forwarder a relayed unwrap: the test
/// forwarder, then the forwarder's unwrap segment, whose first account is the
/// relay's target.
struct RelayedUnwrap {
    test_forwarder: Pubkey,
    forwarder: Pubkey,
    recipient: Pubkey,
    mint: Pubkey,
}

impl Forwarder for RelayedUnwrap {
    fn call_accounts<'a>(
        &'a self,
        _rpc: &'a RpcClient,
        _call: &'a SolanaExternalCall,
    ) -> BoxFuture<'a, anyhow::Result<CallAccounts>> {
        Box::pin(async move {
            let mut segment = vec![AccountMeta::new_readonly(self.test_forwarder, false)];
            segment.extend(build_unwrap_forwarder_accounts(
                &self.forwarder,
                &self.recipient,
                &self.mint,
            ));
            Ok(CallAccounts {
                segment,
                preceding: vec![],
            })
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_forward_call_relayed_by_a_program_the_adapter_invokes() -> anyhow::Result<()> {
    let (mut env, local) = setup::local().await?;
    let forwarder = &local.forwarder;
    let test_forwarder = env.deploy_test_forwarder()?;

    // The escrow holds what the relayed unwrap names, and the recipient has
    // a token account, so only the caller check stands between them.
    let amount = 1_000;
    let escrow = forwarder.escrow_account();
    mint_to(&env, &forwarder.mint, &escrow, amount).await?;
    let recipient = Keypair::new().pubkey();
    let recipient_account = token_account(&env, &recipient, &forwarder.mint).await?;

    let unwrap =
        encode_unwrap_forwarder_input(&forwarder.mint.to_bytes(), amount, &recipient.to_bytes());
    let call = SolanaExternalCall {
        program_id: test_forwarder.to_bytes(),
        instruction_data: relay_input(logic_ref().into(), &unwrap),
        expected_output: vec![RELAY_OK],
        output_mode: OutputMode::ReturnData,
        num_accounts: FORWARDER_UNWRAP_NUM_ACCOUNTS + 1,
    };
    env.protocol_adapter.forwarders.register(
        test_forwarder,
        Arc::new(RelayedUnwrap {
            test_forwarder,
            forwarder: forwarder.program,
            recipient,
            mint: forwarder.mint,
        }),
    );

    let app_data = AppData {
        external_payload: vec![ExpirableBlob {
            blob: bytes_to_words(&call.encode()),
            deletion_criterion: 0,
        }],
        ..AppData::default()
    };
    let action = passthrough::build(1, app_data, passthrough::Overrides::default())?.witnesses;
    let tx = prove_actions(&env, &[action]).await?;

    let before = balances(&env, &[escrow, recipient_account]).await?;
    let settled = execute_tx(&mut env, tx).await;
    // The adapter runs at depth 1, the test forwarder at 2, and the relayed
    // call reaches the forwarder at 3, where it is refused.
    let reached = format!("Program {} invoke [3]", forwarder.program);
    if let Err(error) = &settled {
        anyhow::ensure!(
            format!("{error:?}").contains(&reached),
            "the forwarder was not called through the relay ({reached}): {error:?}"
        );
    }
    expect_integration_panic(Needle::Static("Error Code: UnauthorizedCaller."))(settled)?;
    anyhow::ensure!(
        balances(&env, &[escrow, recipient_account]).await? == before,
        "the relayed unwrap moved tokens"
    );
    Ok(())
}
