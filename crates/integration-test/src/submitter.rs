//! A submitter that tampers with what the forwarder's client supplies.

use std::sync::Arc;

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use anoma_pa_solana_client::forwarders::{CallAccounts, Forwarder, ForwarderError};
use anomapay_spl_token_forwarder_client::submitter::SplTokenForwarder;
use futures::future::BoxFuture;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;

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
        calls: &'a [SolanaExternalCall],
    ) -> BoxFuture<'a, Result<CallAccounts, ForwarderError>> {
        Box::pin(async move {
            let mut accounts = self.submitter.call_accounts(rpc, calls).await?;
            (self.rewrite)(&mut accounts);
            Ok(accounts)
        })
    }
}
