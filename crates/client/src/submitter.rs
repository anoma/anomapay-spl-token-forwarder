//! How a submitter passes the forwarder its calls' accounts: the adapter
//! client's `Forwarder` for this program, through `settlement_calls`.

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use anoma_pa_solana_client::forwarders::{CallAccountsFuture, Forwarder};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;

use crate::settlement::settlement_calls;

/// The submitter's view of the forwarder `program`, which settlements of
/// the adapter `protocol_adapter` call: `payer` pays for the accounts a call
/// needs created. Everything else comes from the call, a wrap's signature
/// included.
pub struct SplTokenForwarderCalls {
    pub program: Pubkey,
    protocol_adapter: Pubkey,
    payer: Pubkey,
}

impl SplTokenForwarderCalls {
    pub fn new(program: Pubkey, protocol_adapter: Pubkey, payer: Pubkey) -> Self {
        Self {
            program,
            protocol_adapter,
            payer,
        }
    }
}

impl Forwarder for SplTokenForwarderCalls {
    fn call_accounts<'a>(
        &'a self,
        _rpc: &'a RpcClient,
        calls: &'a [SolanaExternalCall],
        preceding_start: usize,
    ) -> CallAccountsFuture<'a> {
        Box::pin(async move {
            Ok(settlement_calls(
                &self.program,
                &self.protocol_adapter,
                &self.payer,
                calls,
                preceding_start,
            )?)
        })
    }
}
