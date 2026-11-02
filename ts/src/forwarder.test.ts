import { TOKEN_PROGRAM_ADDRESS } from "@solana-program/token";
import {
  AccountRole,
  address,
  generateKeyPairSigner,
  isSignerRole,
  isWritableRole,
  type AccountMeta,
} from "@solana/kit";
import { describe, expect, it } from "vitest";

import {
  FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS,
  FORWARDER_UNWRAP_NUM_ACCOUNTS,
  FORWARDER_WRAP_NUM_ACCOUNTS,
  forwarderMigrateNumAccounts,
} from "./constants.js";
import {
  buildMigrateForwarderAccounts,
  buildUnwrapForwarderAccounts,
  buildWrapForwarderAccounts,
  forwarderSegmentHead,
  nonceWordIndex,
} from "./forwarder.js";
import { emergencyMigratingIdl, idl } from "./idl.js";
import {
  deriveAdapterNullifierPda,
  deriveAdapterRootMarkerPda,
  deriveEventAuthorityPda,
  deriveForwarderEscrowAuthority,
  deriveMigrationRecordPda,
  deriveNonceBitmapPda,
  derivePaStatePda,
  deriveTokenAccount,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

const newAddress = async () => (await generateKeyPairSigner()).address;
const forwarder = await newAddress();
const mint = await newAddress();

const writable = (accounts: AccountMeta[]) => accounts.map((a) => isWritableRole(a.role));
const signs = (accounts: AccountMeta[]) => accounts.some((a) => isSignerRole(a.role));

describe("forwarder segment builders", () => {
  // The forwarder, then the accounts both builds' IDLs declare for
  // `forward_call`, in their order, with their flags and addresses.
  it("head is the forwarder, then its forward_call accounts", async () => {
    const program = address(idl.address);
    const head = await forwarderSegmentHead(program);
    console.log(head);
    expect(head).toHaveLength(FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS);
    expect(head[0]).toEqual({ address: program, role: AccountRole.READONLY });
    for (const of of [idl, emergencyMigratingIdl]) {
      const declared = of.instructions.find((ix) => ix.name === "forward_call")!.accounts;
      expect(head.length, of.metadata.name).toBe(1 + declared.length);
      for (const [i, account] of declared.entries()) {
        if ("accounts" in account) throw new Error(`forward_call: the account group ${account.name}`);
        const meta = head[1 + i]!;
        const name = `${of.metadata.name} ${account.name}`;
        expect(isWritableRole(meta.role), `${name} writable`).toBe(account.writable ?? false);
        expect(isSignerRole(meta.role), `${name} signer`).toBe(account.signer ?? false);
        // Anchor's `#[event_cpi]` accounts, which the IDL gives no address.
        const expected =
          account.name === "event_authority"
            ? await deriveEventAuthorityPda(program)
            : account.name === "program"
              ? program
              : account.address;
        expect(meta.address, name).toBe(expected);
      }
    }
  });

  it("wrap segment is the head, then the wrap's accounts", async () => {
    const user = await newAddress();
    const accounts = await buildWrapForwarderAccounts(forwarder, user, mint, 300n);
    expect(accounts).toHaveLength(FORWARDER_WRAP_NUM_ACCOUNTS);
    const head = FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS;
    expect(accounts.slice(0, head)).toEqual(await forwarderSegmentHead(forwarder));

    const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarder);
    const tail = accounts.slice(head);
    expect(tail.map((a) => a.address)).toEqual([
      await deriveTokenAccount(user, mint),
      await deriveTokenAccount(escrowAuthority, mint),
      escrowAuthority,
      (await deriveNonceBitmapPda(forwarder, user, 1n))[0], // nonce 300 is in word 1
      TOKEN_PROGRAM_ADDRESS,
    ]);
    expect(writable(tail)).toEqual([true, true, false, true, false]);
    expect(signs(accounts)).toBe(false);
  });

  it("unwrap segment is the head, then the unwrap's accounts", async () => {
    const recipient = await newAddress();
    const accounts = await buildUnwrapForwarderAccounts(forwarder, recipient, mint);
    expect(accounts).toHaveLength(FORWARDER_UNWRAP_NUM_ACCOUNTS);
    const head = FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS;
    expect(accounts.slice(0, head)).toEqual(await forwarderSegmentHead(forwarder));

    const [escrowAuthority] = await deriveForwarderEscrowAuthority(forwarder);
    const tail = accounts.slice(head);
    expect(tail.map((a) => a.address)).toEqual([
      await deriveTokenAccount(escrowAuthority, mint),
      await deriveTokenAccount(recipient, mint),
      escrowAuthority,
      TOKEN_PROGRAM_ADDRESS,
    ]);
    expect(writable(tail)).toEqual([true, true, false, false]);
    expect(signs(accounts)).toBe(false);
  });

  it("migration segment is the head, then the emergency-migrating build's accounts", async () => {
    const adapter = await newAddress();
    const [paState] = await derivePaStatePda(adapter);
    const entries = [1, 2].map((i) => ({
      nullifier: new Uint8Array(32).fill(i),
      commitmentTreeRoot: new Uint8Array(32).fill(10 + i),
    }));
    const accounts = await buildMigrateForwarderAccounts(forwarder, adapter, entries);
    expect(accounts).toHaveLength(forwarderMigrateNumAccounts(2));
    const head = FORWARDER_SEGMENT_HEAD_NUM_ACCOUNTS;
    expect(accounts.slice(0, head)).toEqual(await forwarderSegmentHead(forwarder));
    const entryAccounts = [];
    for (const entry of entries) {
      entryAccounts.push(
        (await deriveAdapterRootMarkerPda(adapter, paState, entry.commitmentTreeRoot))[0],
        (await deriveAdapterNullifierPda(adapter, paState, entry.nullifier))[0],
        (await deriveMigrationRecordPda(forwarder, entry.nullifier))[0],
      );
    }
    const tail = accounts.slice(head);
    expect(tail.map((a) => a.address)).toEqual([paState, (await deriveVulnerableLogicRefsPda(forwarder))[0], ...entryAccounts]);
    // Only the migration records are written.
    expect(writable(tail)).toEqual([false, false, false, false, true, false, false, true]);
    expect(signs(accounts)).toBe(false);
  });

  it("nonce word index covers 256 nonces per word", () => {
    expect(nonceWordIndex(0n)).toBe(0n);
    expect(nonceWordIndex(255n)).toBe(0n);
    expect(nonceWordIndex(256n)).toBe(1n);
  });
});
