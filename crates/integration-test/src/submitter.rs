//! How a submitter passes the SPL token forwarder a call's accounts: the
//! harness's `Forwarder` for this program, through the client's
//! `settlement_calls`.

use std::sync::Arc;

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use anoma_pa_solana_integration_test::forwarders::{CallAccounts, Forwarder};
use anomapay_spl_token_forwarder_client::settlement_calls;
use anyhow::Context;
use futures::future::BoxFuture;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use surfpool_sdk::Pubkey;

/// The submitter's view of the forwarder `program`: `payer` pays for the
/// accounts a call needs created. Everything else comes from the call, a
/// wrap's signature included.
pub struct SplTokenForwarder {
    pub program: Pubkey,
    payer: Pubkey,
}

impl SplTokenForwarder {
    pub fn new(program: Pubkey, payer: Pubkey) -> Self {
        Self { program, payer }
    }
}

impl Forwarder for SplTokenForwarder {
    fn call_accounts<'a>(
        &'a self,
        _rpc: &'a RpcClient,
        call: &'a SolanaExternalCall,
    ) -> BoxFuture<'a, anyhow::Result<CallAccounts>> {
        Box::pin(async move {
            let calls = std::slice::from_ref(call);
            let mut settlement = settlement_calls(&self.program, &self.payer, calls)?;
            Ok(CallAccounts {
                segment: settlement.segments.pop().context("one call, one segment")?,
                preceding: settlement.preceding,
            })
        })
    }
}

/// A submitter passing the forwarder what `rewrite` makes of the accounts and
/// instructions `SplTokenForwarder` supplies: another account in a segment,
/// an instruction dropped, reordered or added.
pub struct Rewritten<F> {
    pub submitter: Arc<SplTokenForwarder>,
    pub rewrite: F,
}

impl<F: Fn(&mut CallAccounts) + Send + Sync> Forwarder for Rewritten<F> {
    fn call_accounts<'a>(
        &'a self,
        rpc: &'a RpcClient,
        call: &'a SolanaExternalCall,
    ) -> BoxFuture<'a, anyhow::Result<CallAccounts>> {
        Box::pin(async move {
            let mut accounts = self.submitter.call_accounts(rpc, call).await?;
            (self.rewrite)(&mut accounts);
            Ok(accounts)
        })
    }
}
