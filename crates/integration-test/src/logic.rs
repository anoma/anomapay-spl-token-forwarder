//! The AnomaPay transfer resource's logics as pa-testkit logic witnesses, as
//! the ERC20 forwarder's tests wrap theirs: pa-testkit's provers constrain or
//! prove them like any other resource logic.

use anoma_pa_testkit::witness::LogicWitness;
use anoma_rm_risc0::Digest;
use anoma_rm_risc0::logic_instance::LogicInstance;
use anoma_rm_risc0::logic_proof::LogicProver;
use anyhow::Context;
use transfer_library::{TOKEN_TRANSFER_ID, TransferLogic};
use transfer_witness::{ForwarderCall, LogicCircuit, TokenTransferWitness};

/// A logic the forwarder serves: the transfer resource's, or one that
/// extends it (the emergency-migrating logic), whose flows the resource's
/// builders build.
pub trait Logic: LogicProver<Witness: LogicCircuit> + From<TransferLogic> + 'static {
    /// The call its ephemeral resources make, which fixes the domain its
    /// owners sign under.
    type Call: ForwarderCall;
}

impl<L, C> Logic for L
where
    L: LogicProver<Witness = TokenTransferWitness<C>> + From<TransferLogic> + 'static,
    C: ForwarderCall,
{
    type Call = C;
}

/// A logic ref no resource has: 32 random bytes.
pub fn random_logic_ref() -> Digest {
    use solana_signer::Signer;
    Digest::from(solana_keypair::Keypair::new().pubkey().to_bytes())
}

/// The logic ref of the transfer resource the forwarder serves.
pub fn logic_ref() -> Digest {
    TOKEN_TRANSFER_ID
}

/// One resource's logic `L`.
pub(crate) struct Witness<L>(pub(crate) L);

impl<L: Logic> LogicWitness for Witness<L> {
    fn verifying_key(&self) -> Digest {
        L::verifying_key()
    }

    fn constrain(&self) -> anyhow::Result<LogicInstance> {
        self.0
            .witness()
            .constrain()
            .map_err(|e| anyhow::anyhow!("{e:?}"))
            .context("the logic refuses its witness")
    }

    fn witness_to_vec(&self) -> anyhow::Result<Vec<u32>> {
        risc0_zkvm::serde::to_vec(self.0.witness()).context("failed to serialize the witness")
    }

    fn proving_key(&self) -> Vec<u8> {
        L::proving_key().to_vec()
    }
}
