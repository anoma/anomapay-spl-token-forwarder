/**
 * Instruction builders for the SPL token forwarder: the one form of each
 * instruction the operator scripts send. Builders return an Anchor method
 * builder; callers add signers and send.
 */
import { Program } from "@anchor-lang/core";
import { AccountMeta, ComputeBudgetProgram, PublicKey } from "@solana/web3.js";
import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { SplTokenForwarder } from "../target/types/spl_token_forwarder";
import { deriveForwarderEscrowAuthority } from "../ts/src/pda";

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

/** A mint's escrow: the forwarder's escrow authority and its associated token account for the mint. */
export function escrowAccounts(
  forwarderProgramId: PublicKey,
  mint: PublicKey,
): { escrowAuthority: PublicKey; escrowAta: PublicKey } {
  const [escrowAuthority] = deriveForwarderEscrowAuthority(forwarderProgramId);
  return { escrowAuthority, escrowAta: getAssociatedTokenAddressSync(mint, escrowAuthority, true) };
}

/**
 * The accounts of an escrow release, in the order the program reads them:
 * the unwrap's remaining accounts after the segment head, and the whole of
 * forward_emergency_call's.
 */
export function escrowTransferAccounts(
  escrowAta: PublicKey,
  recipientAta: PublicKey,
  escrowAuthority: PublicKey,
): AccountMeta[] {
  return [
    { pubkey: escrowAta, isSigner: false, isWritable: true },
    { pubkey: recipientAta, isSigner: false, isWritable: true },
    { pubkey: escrowAuthority, isSigner: false, isWritable: false },
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
  ];
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

/** `forward_emergency_call` by `caller`; callers add signers and send. */
export function emergencyWithdraw(
  forwarder: Program<SplTokenForwarder>,
  paState: PublicKey,
  caller: PublicKey,
  withdrawal: { mint: PublicKey; amount: bigint; recipient: PublicKey },
  accounts: { escrowAta: PublicKey; recipientAta: PublicKey },
) {
  return forwarder.methods
    .forwardEmergencyCall(encodeUnwrapInput(withdrawal.mint, withdrawal.amount, withdrawal.recipient))
    .accountsPartial({ caller, paState })
    .remainingAccounts(
      escrowTransferAccounts(
        accounts.escrowAta,
        accounts.recipientAta,
        deriveForwarderEscrowAuthority(forwarder.programId)[0],
      ),
    );
}
