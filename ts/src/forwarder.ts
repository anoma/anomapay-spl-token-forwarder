// Builders for the forwarder's CPI account segments inside a settle
// transaction's remaining accounts, which no IDL lists. Ordering is owned by
// the forwarder program; integrators must use these builders rather than
// hand-rolling the slice. The forwarder's own instructions are built by its
// instances (`instance.ts`).

import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { type AccountMeta, SYSVAR_INSTRUCTIONS_PUBKEY, type PublicKey } from "@solana/web3.js";

import { idl, idlConstant } from "./idl.js";
import {
  deriveAdapterNullifierPda,
  deriveAdapterRootMarkerPda,
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveMigrationRecordPda,
  deriveNonceBitmapPda,
  derivePaStatePda,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

/**
 * Nonces per nonce-bitmap account, the IDL's `NONCES_PER_WORD`. The bitmap for
 * `nonce` is the one for word `nonce / NONCES_PER_WORD`.
 */
export const NONCES_PER_WORD = BigInt(idlConstant(idl, "NONCES_PER_WORD"));

/** The nonce-bitmap word a wrap nonce falls in. */
export function nonceWordIndex(nonce: bigint): bigint {
  return nonce / NONCES_PER_WORD;
}

/**
 * The accounts every forwarder CPI segment starts with: the forwarder (the
 * call's target), its config, the instructions sysvar, then the event
 * authority and the forwarder again, which its CPI events need.
 */
function forwarderSegmentHead(forwarderProgram: PublicKey): AccountMeta[] {
  return [
    { pubkey: forwarderProgram, isSigner: false, isWritable: false },
    { pubkey: deriveForwarderConfigPda(forwarderProgram)[0], isSigner: false, isWritable: false },
    { pubkey: SYSVAR_INSTRUCTIONS_PUBKEY, isSigner: false, isWritable: false },
    { pubkey: deriveEventAuthorityPda(forwarderProgram), isSigner: false, isWritable: false },
    { pubkey: forwarderProgram, isSigner: false, isWritable: false },
  ];
}

/**
 * Build the wrap forwarder CPI segment: `[forwarder_program, config,
 * ix_sysvar, event_authority, forwarder_program, user_ata, escrow_ata,
 * escrow_authority, nonce_bitmap_pda, token_program]`. The nonce bitmap must
 * already exist (the instance's `initNonceBitmapIx`).
 */
export function buildWrapForwarderAccounts(
  forwarderProgram: PublicKey,
  user: PublicKey,
  tokenMint: PublicKey,
  nonce: bigint,
): AccountMeta[] {
  const [escrowAuthority] = deriveForwarderEscrowAuthority(forwarderProgram);
  const [nonceBitmapPda] = deriveNonceBitmapPda(forwarderProgram, user, nonceWordIndex(nonce));
  return [
    ...forwarderSegmentHead(forwarderProgram),
    { pubkey: getAssociatedTokenAddressSync(tokenMint, user, true), isSigner: false, isWritable: true },
    { pubkey: getAssociatedTokenAddressSync(tokenMint, escrowAuthority, true), isSigner: false, isWritable: true },
    { pubkey: escrowAuthority, isSigner: false, isWritable: false },
    { pubkey: nonceBitmapPda, isSigner: false, isWritable: true },
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
  ];
}

/** The accounts a migration's segment reads of one entry: its nullifier and the root it is proven against. */
export interface MigrateEntryAccounts {
  nullifier: Uint8Array;
  commitmentTreeRoot: Uint8Array;
}

/**
 * Build the emergency-migrating build's migration CPI segment through the
 * adapter `protocolAdapter`: the segment head, the adapter's state and the
 * vulnerable logic refs, then for each entry the adapter's marker of its root,
 * the adapter's marker of its nullifier and its migration record, which must
 * already exist (the instance's `initMigrationRecordIx`).
 */
export function buildMigrateForwarderAccounts(
  forwarderProgram: PublicKey,
  protocolAdapter: PublicKey,
  entries: MigrateEntryAccounts[],
): AccountMeta[] {
  const [paState] = derivePaStatePda(protocolAdapter);
  return [
    ...forwarderSegmentHead(forwarderProgram),
    { pubkey: paState, isSigner: false, isWritable: false },
    { pubkey: deriveVulnerableLogicRefsPda(forwarderProgram)[0], isSigner: false, isWritable: false },
    ...entries.flatMap((entry) => [
      {
        pubkey: deriveAdapterRootMarkerPda(protocolAdapter, paState, entry.commitmentTreeRoot)[0],
        isSigner: false,
        isWritable: false,
      },
      {
        pubkey: deriveAdapterNullifierPda(protocolAdapter, paState, entry.nullifier)[0],
        isSigner: false,
        isWritable: false,
      },
      { pubkey: deriveMigrationRecordPda(forwarderProgram, entry.nullifier)[0], isSigner: false, isWritable: true },
    ]),
  ];
}

/**
 * Build the unwrap forwarder CPI segment: `[forwarder_program, config,
 * ix_sysvar, event_authority, forwarder_program, escrow_ata, recipient_ata,
 * escrow_authority, token_program]`.
 */
export function buildUnwrapForwarderAccounts(
  forwarderProgram: PublicKey,
  recipient: PublicKey,
  tokenMint: PublicKey,
): AccountMeta[] {
  const [escrowAuthority] = deriveForwarderEscrowAuthority(forwarderProgram);
  return [
    ...forwarderSegmentHead(forwarderProgram),
    { pubkey: getAssociatedTokenAddressSync(tokenMint, escrowAuthority, true), isSigner: false, isWritable: true },
    { pubkey: getAssociatedTokenAddressSync(tokenMint, recipient, true), isSigner: false, isWritable: true },
    { pubkey: escrowAuthority, isSigner: false, isWritable: false },
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
  ];
}

