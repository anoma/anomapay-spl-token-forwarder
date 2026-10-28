/**
 * Instruction builders for the SPL token forwarder: the one form of each
 * instruction the operator scripts send. Builders return an Anchor method
 * builder; callers add signers and send.
 */
import { Program } from "@anchor-lang/core";
import type { Address } from "@solana/kit";
import { SplTokenForwarder } from "../target/types/spl_token_forwarder";

/**
 * The forwarder's `initialize`, signed by `authority`, the program's upgrade
 * authority, which it hands to the program's upgrade authority PDA: it makes
 * `initialOwner` the owner. Callers add signers and send.
 */
export function initializeForwarder(
  forwarder: Program<SplTokenForwarder>,
  adapterProgramId: Address,
  logicRef: number[],
  initialOwner: Address,
  authority: Address,
) {
  return forwarder.methods.initialize(adapterProgramId, logicRef, initialOwner).accountsPartial({ authority });
}

/**
 * Rotate the forwarder config's logic ref, once per build that raises
 * CONFIG_VERSION, after upgrading the program to that build. `authority`
 * must be the forwarder's owner.
 */
export function reinitializeForwarder(forwarder: Program<SplTokenForwarder>, authority: Address, logicRef: number[]) {
  return forwarder.methods.reinitialize(logicRef).accountsPartial({ authority });
}
