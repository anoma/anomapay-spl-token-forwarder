//! How a submitter passes the forwarder its calls' accounts: the adapter
//! client's `Forwarder` for this program, through `settlement_calls`.

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use anoma_pa_solana_client::forwarders::{CallAccountsFuture, Forwarder};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;

use crate::settlement::settlement_calls;

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
        calls: &'a [SolanaExternalCall],
        preceding_start: usize,
    ) -> CallAccountsFuture<'a> {
        Box::pin(async move {
            Ok(settlement_calls(
                &self.program,
                &self.payer,
                calls,
                preceding_start,
            )?)
        })
    }
}
