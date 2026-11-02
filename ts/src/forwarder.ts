// Builders for the forwarder's CPI account segments inside a settle
// transaction's remaining accounts: the head the IDL declares for
// `forward_call`, then the accounts of the call, which no IDL lists. Ordering
// is owned by the forwarder program; integrators must use these builders
// rather than hand-rolling the slice. The forwarder's own instructions are
// built by its instances (`contract.ts`).

import { TOKEN_PROGRAM_ADDRESS } from "@solana-program/token";
import { AccountRole, type AccountMeta, type Address } from "@solana/kit";

import { FORWARD_CALL_ACCOUNTS } from "./constants.js";
import { accountRole, idl, idlConstant } from "./idl.js";
import {
  deriveAdapterNullifierPda,
  deriveAdapterRootMarkerPda,
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveMigrationRecordPda,
  deriveNonceBitmapPda,
  derivePaStatePda,
  deriveTokenAccount,
  deriveVulnerableLogicRefsPda,
  SYSVAR_INSTRUCTIONS_ADDRESS,
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

const readonly = (address: Address): AccountMeta => ({ address, role: AccountRole.READONLY });
const writable = (address: Address): AccountMeta => ({ address, role: AccountRole.WRITABLE });

/**
 * The accounts every forwarder CPI segment starts with: the forwarder, the
 * program the adapter calls, then the accounts the IDL declares for its
 * `forward_call`, in the IDL's order and with its flags.
 */
export async function forwarderSegmentHead(forwarderProgram: Address): Promise<AccountMeta[]> {
  const named: Record<string, Address> = {
    config: (await deriveForwarderConfigPda(forwarderProgram))[0],
    ix_sysvar: SYSVAR_INSTRUCTIONS_ADDRESS,
    event_authority: await deriveEventAuthorityPda(forwarderProgram),
    program: forwarderProgram,
  };
  return [
    readonly(forwarderProgram),
    ...FORWARD_CALL_ACCOUNTS.map((account): AccountMeta => {
      if ("accounts" in account) throw new Error(`forward_call: the account group ${account.name} is not supported`);
      const given = named[account.name];
      if (given === undefined) throw new Error(`forward_call takes an account ${account.name} the head does not name`);
      return { address: given, role: accountRole(account.signer ?? false, account.writable ?? false) };
    }),
  ];
}

/**
 * Build the wrap forwarder CPI segment: `[forwarder_program, config,
 * ix_sysvar, event_authority, forwarder_program, user_ata, escrow_ata,
 * escrow_authority, nonce_bitmap_pda, token_program]`. The nonce bitmap must
 * already exist (the instance's `initNonceBitmapIx`).
 */
export async function buildWrapForwarderAccounts(
  forwarderProgram: Address,
  user: Address,
  tokenMint: Address,
  nonce: bigint,
): Promise<AccountMeta[]> {
  const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarderProgram);
  const [nonceBitmapPda] = await deriveNonceBitmapPda(forwarderProgram, user, nonceWordIndex(nonce));
  return [
    ...(await forwarderSegmentHead(forwarderProgram)),
    writable(await deriveTokenAccount(user, tokenMint)),
    writable(await deriveTokenAccount(escrowAuthority, tokenMint)),
    readonly(escrowAuthority),
    writable(nonceBitmapPda),
    readonly(TOKEN_PROGRAM_ADDRESS),
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
export async function buildMigrateForwarderAccounts(
  forwarderProgram: Address,
  protocolAdapter: Address,
  entries: MigrateEntryAccounts[],
): Promise<AccountMeta[]> {
  const [paState] = await derivePaStatePda(protocolAdapter);
  const entryAccounts = await Promise.all(
    entries.map(async (entry) => [
      readonly((await deriveAdapterRootMarkerPda(protocolAdapter, paState, entry.commitmentTreeRoot))[0]),
      readonly((await deriveAdapterNullifierPda(protocolAdapter, paState, entry.nullifier))[0]),
      writable((await deriveMigrationRecordPda(forwarderProgram, entry.nullifier))[0]),
    ]),
  );
  return [
    ...(await forwarderSegmentHead(forwarderProgram)),
    readonly(paState),
    readonly((await deriveVulnerableLogicRefsPda(forwarderProgram))[0]),
    ...entryAccounts.flat(),
  ];
}

/**
 * Build the unwrap forwarder CPI segment: `[forwarder_program, config,
 * ix_sysvar, event_authority, forwarder_program, escrow_ata, recipient_ata,
 * escrow_authority, token_program]`.
 */
export async function buildUnwrapForwarderAccounts(
  forwarderProgram: Address,
  recipient: Address,
  tokenMint: Address,
): Promise<AccountMeta[]> {
  const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarderProgram);
  return [
    ...(await forwarderSegmentHead(forwarderProgram)),
    writable(await deriveTokenAccount(escrowAuthority, tokenMint)),
    writable(await deriveTokenAccount(recipient, tokenMint)),
    readonly(escrowAuthority),
    readonly(TOKEN_PROGRAM_ADDRESS),
  ];
}
