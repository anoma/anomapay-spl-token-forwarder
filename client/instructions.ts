/**
 * Instruction builders for the SPL token forwarder: the one form of each
 * instruction the operator scripts send. Builders return an Anchor method
 * builder; callers add signers and send.
 */
import { Program } from "@anchor-lang/core";
import { ComputeBudgetProgram, PublicKey } from "@solana/web3.js";
import { SplTokenForwarder } from "../target/types/spl_token_forwarder";
import { escrowReleaseAccounts } from "../ts/src/forwarder";

/**
 * The most compute units a transaction may use. `upgrade` runs under it,
 * since it hashes the whole buffer (sha256's cost grows with the length)
 * before the loader's own work.
 */
export const MAX_COMPUTE_UNIT_LIMIT = 1_400_000;

/**
 * The 72-byte unwrap operand (token_mint, amount u64 LE, recipient): the
 * whole input of forward_emergency_call, and forward_call's input after
 * the op-code byte.
 */
export function encodeUnwrapInput(tokenMint: PublicKey, amount: bigint, recipient: PublicKey): Buffer {
  const operand = Buffer.alloc(72);
  tokenMint.toBuffer().copy(operand, 0);
  operand.writeBigUInt64LE(amount, 32);
  recipient.toBuffer().copy(operand, 40);
  return operand;
}

/**
 * The forwarder's `initialize`, signed by `authority`, the program's upgrade
 * authority, which it hands to the program's upgrade authority PDA: it makes
 * `initialOwner` the owner. Callers add signers and send.
 */
export function initializeForwarder(
  forwarder: Program<SplTokenForwarder>,
  adapterProgramId: PublicKey,
  logicRef: number[],
  committee: PublicKey,
  initialOwner: PublicKey,
  authority: PublicKey,
) {
  return forwarder.methods
    .initialize(adapterProgramId, logicRef, committee, initialOwner)
    .accountsPartial({ authority });
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

/**
 * `forward_emergency_call` by `caller`: `withdrawal.amount` of
 * `withdrawal.mint` from escrow to `withdrawal.recipient`'s token account,
 * which must exist. Callers add signers and send.
 */
export function emergencyWithdraw(
  forwarder: Program<SplTokenForwarder>,
  paState: PublicKey,
  caller: PublicKey,
  withdrawal: { mint: PublicKey; amount: bigint; recipient: PublicKey },
) {
  return forwarder.methods
    .forwardEmergencyCall(encodeUnwrapInput(withdrawal.mint, withdrawal.amount, withdrawal.recipient))
    .accountsPartial({ caller, paState })
    .remainingAccounts(escrowReleaseAccounts(forwarder.programId, withdrawal.recipient, withdrawal.mint));
}
