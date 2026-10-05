//! The AnomaPay transfer resource's logic as a pa-testkit logic witness, as
//! the ERC20 forwarder's tests wrap theirs: pa-testkit's provers constrain or
//! prove it like any other resource logic.

use anoma_pa_testkit::witness::LogicWitness;
use anoma_rm_risc0::Digest;
use anoma_rm_risc0::logic_instance::LogicInstance;
use anyhow::Context;
use transfer_library::{TOKEN_TRANSFER_ELF, TOKEN_TRANSFER_ID, TransferLogic};
use transfer_witness::LogicCircuit;

/// The logic ref of the transfer resource the forwarder serves.
pub fn logic_ref() -> Digest {
    TOKEN_TRANSFER_ID
}

/// One resource's transfer logic.
pub(crate) struct Witness(pub(crate) TransferLogic);

impl LogicWitness for Witness {
    fn verifying_key(&self) -> Digest {
        TOKEN_TRANSFER_ID
    }

    fn constrain(&self) -> anyhow::Result<LogicInstance> {
        self.0
            .witness
            .constrain()
            .map_err(|e| anyhow::anyhow!("{e:?}"))
            .context("the transfer logic refuses its witness")
    }

    fn witness_to_vec(&self) -> anyhow::Result<Vec<u32>> {
        risc0_zkvm::serde::to_vec(&self.0.witness)
            .context("failed to serialize the transfer witness")
    }

    fn proving_key(&self) -> Vec<u8> {
        TOKEN_TRANSFER_ELF.to_vec()
    }
}
