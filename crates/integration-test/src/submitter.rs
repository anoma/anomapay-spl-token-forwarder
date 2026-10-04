//! How a submitter passes the SPL token forwarder a call's accounts: the
//! harness's `Forwarder` for this program.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anoma_pa_solana_client::external_call::SolanaExternalCall;
use anoma_pa_solana_integration_test::forwarders::{CallAccounts, Forwarder};
use anomapay_spl_token_forwarder_client::{
    ForwarderInput, build_unwrap_forwarder_accounts, build_wrap_forwarder_accounts,
    create_ata_idempotent_ix, decode_forwarder_input, derive_nonce_bitmap_pda,
    init_nonce_bitmap_ix, nonce_word_index,
};
use anyhow::Context;
use futures::future::BoxFuture;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use surfpool_sdk::Pubkey;

use crate::fixtures::WrapAuthorization;

/// The submitter's view of the forwarder `program`: the authorizations its
/// users signed for their wraps, which a wrap's settlement carries in an
/// ed25519 instruction, and the payer of the accounts a call needs created.
pub struct SplTokenForwarder {
    pub program: Pubkey,
    payer: Pubkey,
    authorizations: Mutex<HashMap<(Pubkey, u64), WrapAuthorization>>,
}

impl SplTokenForwarder {
    pub fn new(program: Pubkey, payer: Pubkey) -> Self {
        Self {
            program,
            payer,
            authorizations: Mutex::new(HashMap::new()),
        }
    }

    /// Records `authorization` as what `user` signed for the wrap with
    /// `nonce`: the ed25519 instruction its settlement carries.
    pub fn authorize(&self, user: Pubkey, nonce: u64, authorization: WrapAuthorization) {
        self.authorizations
            .lock()
            .expect("the authorizations lock is not poisoned")
            .insert((user, nonce), authorization);
    }
}

impl Forwarder for SplTokenForwarder {
    fn call_accounts<'a>(
        &'a self,
        rpc: &'a RpcClient,
        call: &'a SolanaExternalCall,
    ) -> BoxFuture<'a, anyhow::Result<CallAccounts>> {
        Box::pin(async move {
            match decode_forwarder_input(&call.instruction_data)? {
                ForwarderInput::Wrap(wrap) => {
                    let user = Pubkey::new_from_array(wrap.user);
                    let mint = Pubkey::new_from_array(wrap.token_mint);
                    let authorization = self
                        .authorizations
                        .lock()
                        .expect("the authorizations lock is not poisoned")
                        .get(&(user, wrap.nonce))
                        .cloned()
                        .with_context(|| {
                            format!("{user} authorized no wrap with nonce {}", wrap.nonce)
                        })?;
                    let mut preceding = vec![
                        solana_ed25519_program::new_ed25519_instruction_with_signature(
                            &authorization.message,
                            &authorization.signature,
                            &authorization.signer.to_bytes(),
                        ),
                    ];
                    // A wrap needs the bitmap of its nonce's word; the
                    // adapter's CPI carries no signer that could create it.
                    let word = nonce_word_index(wrap.nonce);
                    let (bitmap, _) = derive_nonce_bitmap_pda(&self.program, &user, word);
                    let existing = rpc
                        .get_account_with_commitment(&bitmap, rpc.commitment())
                        .await
                        .with_context(|| format!("failed to read the nonce bitmap {bitmap}"))?
                        .value;
                    if existing.is_none() {
                        preceding.push(init_nonce_bitmap_ix(
                            &self.program,
                            &self.payer,
                            &user,
                            word,
                        ));
                    }
                    Ok(CallAccounts {
                        segment: build_wrap_forwarder_accounts(
                            &self.program,
                            &user,
                            &mint,
                            wrap.nonce,
                        ),
                        preceding,
                    })
                }
                ForwarderInput::Unwrap(unwrap) => {
                    let recipient = Pubkey::new_from_array(unwrap.recipient);
                    let mint = Pubkey::new_from_array(unwrap.token_mint);
                    Ok(CallAccounts {
                        segment: build_unwrap_forwarder_accounts(&self.program, &recipient, &mint),
                        // The forwarder's transfer needs the recipient's
                        // token account to exist.
                        preceding: vec![create_ata_idempotent_ix(&self.payer, &recipient, &mint)],
                    })
                }
            }
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
