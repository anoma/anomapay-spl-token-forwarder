import { findAssociatedTokenPda, TOKEN_PROGRAM_ADDRESS } from "@solana-program/token";
import { AccountRole, generateKeyPairSigner, type AccountMeta, type Address } from "@solana/kit";
import { SYSVAR_INSTRUCTIONS_ADDRESS } from "@solana/sysvars";
import { describe, expect, it } from "vitest";

import {
  FORWARDER_UNWRAP_NUM_ACCOUNTS,
  FORWARDER_WRAP_NUM_ACCOUNTS,
  forwarderMigrateNumAccounts,
} from "./constants.js";
import {
  buildMigrateForwarderAccounts,
  buildUnwrapForwarderAccounts,
  buildWrapForwarderAccounts,
  nonceWordIndex,
} from "./forwarder.js";
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
 * `owner`'s token account for `mint` as the SPL Associated Token Account
 * program derives it: `[owner, token program, mint]`. The owner may be a PDA.
 */
const tokenAccount = async (owner: Address, mint: Address) =>
  (await findAssociatedTokenPda({ owner, mint, tokenProgram: TOKEN_PROGRAM_ADDRESS }))[0];

const newAddress = async () => (await generateKeyPairSigner()).address;
const forwarder = await newAddress();
const mint = await newAddress();

const writable = (accounts: AccountMeta[]) => accounts.map((a) => a.role === AccountRole.WRITABLE);
const signs = (accounts: AccountMeta[]) =>
  accounts.some((a) => a.role === AccountRole.READONLY_SIGNER || a.role === AccountRole.WRITABLE_SIGNER);

describe("forwarder segment builders", () => {
  it("wrap segment is the forwarder layout", async () => {
    const user = await newAddress();
    const accounts = await buildWrapForwarderAccounts(forwarder, user, mint, 300n);
    expect(accounts).toHaveLength(FORWARDER_WRAP_NUM_ACCOUNTS);

    const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarder);
    expect(accounts.map((a) => a.address)).toEqual([
      forwarder,
      (await deriveForwarderConfigPda(forwarder))[0],
      SYSVAR_INSTRUCTIONS_ADDRESS,
      await deriveEventAuthorityPda(forwarder),
      forwarder,
      await tokenAccount(user, mint),
      await tokenAccount(escrowAuthority, mint),
      escrowAuthority,
      (await deriveNonceBitmapPda(forwarder, user, 1n))[0], // nonce 300 is in word 1
      TOKEN_PROGRAM_ADDRESS,
    ]);
    expect(writable(accounts)).toEqual([false, false, false, false, false, true, true, false, true, false]);
    expect(signs(accounts)).toBe(false);
  });

  it("unwrap segment is the forwarder layout", async () => {
    const recipient = await newAddress();
    const accounts = await buildUnwrapForwarderAccounts(forwarder, recipient, mint);
    expect(accounts).toHaveLength(FORWARDER_UNWRAP_NUM_ACCOUNTS);

    const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarder);
    expect(accounts.map((a) => a.address)).toEqual([
      forwarder,
      (await deriveForwarderConfigPda(forwarder))[0],
      SYSVAR_INSTRUCTIONS_ADDRESS,
      await deriveEventAuthorityPda(forwarder),
      forwarder,
      await tokenAccount(escrowAuthority, mint),
      await tokenAccount(recipient, mint),
      escrowAuthority,
      TOKEN_PROGRAM_ADDRESS,
    ]);
    expect(writable(accounts)).toEqual([false, false, false, false, false, true, true, false, false]);
    expect(signs(accounts)).toBe(false);
  });

  it("migration segment is the emergency-migrating build's layout", async () => {
    const adapter = await newAddress();
    const [paState] = await derivePaStatePda(adapter);
    const entries = [1, 2].map((i) => ({
      nullifier: new Uint8Array(32).fill(i),
      commitmentTreeRoot: new Uint8Array(32).fill(10 + i),
    }));
    const accounts = await buildMigrateForwarderAccounts(forwarder, adapter, entries);
    expect(accounts).toHaveLength(forwarderMigrateNumAccounts(2));
    const entryAccounts = [];
    for (const entry of entries) {
      entryAccounts.push(
        (await deriveAdapterRootMarkerPda(adapter, paState, entry.commitmentTreeRoot))[0],
        (await deriveAdapterNullifierPda(adapter, paState, entry.nullifier))[0],
        (await deriveMigrationRecordPda(forwarder, entry.nullifier))[0],
      );
    }
    expect(accounts.map((a) => a.address)).toEqual([
      forwarder,
      (await deriveForwarderConfigPda(forwarder))[0],
      SYSVAR_INSTRUCTIONS_ADDRESS,
      await deriveEventAuthorityPda(forwarder),
      forwarder,
      paState,
      (await deriveVulnerableLogicRefsPda(forwarder))[0],
      ...entryAccounts,
    ]);
    // Only the migration records are written.
    expect(writable(accounts)).toEqual([
      false, false, false, false, false, false, false, false, false, true, false, false, true,
    ]);
    expect(signs(accounts)).toBe(false);
  });

  it("nonce word index covers 256 nonces per word", () => {
    expect(nonceWordIndex(0n)).toBe(0n);
    expect(nonceWordIndex(255n)).toBe(0n);
    expect(nonceWordIndex(256n)).toBe(1n);
  });
});
