/**
 * Instruction builders for the SPL token forwarder: the one form of each
 * instruction the operator scripts send. Builders return an Anchor method
 * builder; callers add signers and send.
 */
import { Program } from "@anchor-lang/core";
import { ComputeBudgetProgram, PublicKey } from "@solana/web3.js";
import { SplTokenForwarder } from "../target/types/spl_token_forwarder";

/**
 * The most compute units a transaction may use. `upgrade` runs under it,
 * since it hashes the whole buffer (sha256's cost grows with the length)
 * before the loader's own work.
 */
export const MAX_COMPUTE_UNIT_LIMIT = 1_400_000;

/**
 * The forwarder's `initialize`, signed by `authority`, the program's upgrade
 * authority, which it hands to the program's upgrade authority PDA: it makes
 * `initialOwner` the owner. Callers add signers and send.
 */
export function initializeForwarder(
  forwarder: Program<SplTokenForwarder>,
  adapterProgramId: PublicKey,
  logicRef: number[],
  initialOwner: PublicKey,
  authority: PublicKey,
) {
  return forwarder.methods.initialize(adapterProgramId, logicRef, initialOwner).accountsPartial({ authority });
}

/**
 * Rotate the forwarder config's logic ref, once per build that raises
 * CONFIG_VERSION, after upgrading the program to that build. `authority`
 * must be the forwarder's owner.
 */
export function reinitializeForwarder(forwarder: Program<SplTokenForwarder>, authority: PublicKey, logicRef: number[]) {
  return forwarder.methods.reinitialize(logicRef).accountsPartial({ authority });
}

/**
 * The forwarder's `upgrade` by its owner: the program's code becomes
 * `buffer`'s, a loader buffer the owner wrote; the buffer's rent goes to
 * `spill`.
 */
export function upgradeForwarder(
  forwarder: Program<SplTokenForwarder>,
  authority: PublicKey,
  buffer: PublicKey,
  spill: PublicKey,
) {
  return forwarder.methods
    .upgrade()
    .accountsPartial({ authority, buffer, spill })
    .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: MAX_COMPUTE_UNIT_LIMIT })]);
}
