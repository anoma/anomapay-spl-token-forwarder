// Builders for the forwarder's CPI account segments inside a settle
// transaction's remaining accounts, and for the forwarder's own
// `init_nonce_bitmap` and `init_migration_record` instructions. Ordering is owned by the forwarder program;
// integrators must use these builders rather than hand-rolling the slice.

import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  type AccountMeta,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  SystemProgram,
  TransactionInstruction,
  type PublicKey,
} from "@solana/web3.js";

import { anchorDiscriminator } from "./discriminator.js";
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
 * Nonces per nonce-bitmap account. The bitmap for `nonce` is the one for word
 * `nonce / NONCES_PER_WORD`.
 */
export const NONCES_PER_WORD = 256n;

/** The nonce-bitmap word a wrap nonce falls in. */
export function nonceWordIndex(nonce: bigint): bigint {
  return nonce / NONCES_PER_WORD;
}

/**
 * Build the forwarder's permissionless `init_nonce_bitmap` instruction, which
 * creates `user`'s bitmap for `wordIndex` with `payer` funding the rent.
 *
 * A wrap needs the bitmap for its nonce's word to exist, so a settlement whose
 * word has no bitmap yet (the account at
 * `deriveNonceBitmapPda(forwarderProgram, user, wordIndex)` is absent) carries
 * this instruction before the settle instruction. It fits in the settlement
 * transaction after the ed25519 instruction.
 */
export function initNonceBitmapIx(
  forwarderProgram: PublicKey,
  payer: PublicKey,
  user: PublicKey,
  wordIndex: bigint,
): TransactionInstruction {
  const [nonceBitmapPda] = deriveNonceBitmapPda(forwarderProgram, user, wordIndex);
  const data = new Uint8Array(8 + 32 + 8);
  data.set(anchorDiscriminator("global", "init_nonce_bitmap"), 0);
  data.set(user.toBytes(), 8);
  new DataView(data.buffer).setBigUint64(40, wordIndex, true);
  return new TransactionInstruction({
    programId: forwarderProgram,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: nonceBitmapPda, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.from(data),
  });
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
    { pubkey: deriveEventAuthorityPda(forwarderProgram)[0], isSigner: false, isWritable: false },
    { pubkey: forwarderProgram, isSigner: false, isWritable: false },
  ];
}

/**
 * Build the wrap forwarder CPI segment: `[forwarder_program, config,
 * ix_sysvar, event_authority, forwarder_program, user_ata, escrow_ata,
 * escrow_authority, nonce_bitmap_pda, token_program]`. The nonce bitmap must
 * already exist (`initNonceBitmapIx`).
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
 * Build the emergency-migrating build's permissionless `init_migration_record`
 * instruction, which creates the migration record of the resource with
 * `nullifier`, `payer` funding the rent, and does nothing when it exists. A
 * migration needs every entry's record, so its settlement carries this before
 * the settle instruction.
 */
export function initMigrationRecordIx(
  forwarderProgram: PublicKey,
  payer: PublicKey,
  nullifier: Uint8Array,
): TransactionInstruction {
  const [record] = deriveMigrationRecordPda(forwarderProgram, nullifier);
  const data = new Uint8Array(8 + 32);
  data.set(anchorDiscriminator("global", "init_migration_record"), 0);
  data.set(nullifier, 8);
  return new TransactionInstruction({
    programId: forwarderProgram,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: record, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: Buffer.from(data),
  });
}

/**
 * Build the emergency-migrating build's migration CPI segment through the
 * adapter `protocolAdapter`: the segment head, the adapter's state and the
 * vulnerable logic refs, then for each entry the adapter's marker of its root,
 * the adapter's marker of its nullifier and its migration record, which must
 * already exist (`initMigrationRecordIx`).
 */
export function buildMigrateForwarderAccounts(
  forwarderProgram: PublicKey,
  protocolAdapter: PublicKey,
  entries: MigrateEntryAccounts[],
): AccountMeta[] {
  return [
    ...forwarderSegmentHead(forwarderProgram),
    { pubkey: derivePaStatePda(protocolAdapter)[0], isSigner: false, isWritable: false },
    { pubkey: deriveVulnerableLogicRefsPda(forwarderProgram)[0], isSigner: false, isWritable: false },
    ...entries.flatMap((entry) => [
      {
        pubkey: deriveAdapterRootMarkerPda(protocolAdapter, entry.commitmentTreeRoot)[0],
        isSigner: false,
        isWritable: false,
      },
      { pubkey: deriveAdapterNullifierPda(protocolAdapter, entry.nullifier)[0], isSigner: false, isWritable: false },
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

