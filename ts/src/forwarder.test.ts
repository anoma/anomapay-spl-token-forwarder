import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { Keypair, type PublicKey, SYSVAR_INSTRUCTIONS_PUBKEY, SystemProgram } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import emergencyMigratingIdl from "../../crates/client/idl/emergency_migrating_spl_token_forwarder.json";
import forwarderIdl from "../../crates/client/idl/spl_token_forwarder.json";
import {
  FORWARDER_UNWRAP_NUM_ACCOUNTS,
  FORWARDER_WRAP_NUM_ACCOUNTS,
  forwarderMigrateNumAccounts,
} from "./constants.js";
import {
  buildMigrateForwarderAccounts,
  buildUnwrapForwarderAccounts,
  buildWrapForwarderAccounts,
  initMigrationRecordIx,
  initNonceBitmapIx,
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

/** `owner`'s token account for `mint`; the owner may be a PDA. */
const tokenAccount = (owner: PublicKey, mint: PublicKey) => getAssociatedTokenAddressSync(mint, owner, true);

const forwarder = Keypair.generate().publicKey;
const mint = Keypair.generate().publicKey;

describe("forwarder segment builders", () => {
  it("wrap segment is the forwarder layout", () => {
    const user = Keypair.generate().publicKey;
    const accounts = buildWrapForwarderAccounts(forwarder, user, mint, 300n);
    expect(accounts).toHaveLength(FORWARDER_WRAP_NUM_ACCOUNTS);

    const [escrowAuthority] = deriveForwarderEscrowAuthority(forwarder);
    expect(accounts.map((a) => a.pubkey.toBase58())).toEqual(
      [
        forwarder,
        deriveForwarderConfigPda(forwarder)[0],
        SYSVAR_INSTRUCTIONS_PUBKEY,
        deriveEventAuthorityPda(forwarder)[0],
        forwarder,
        tokenAccount(user, mint),
        tokenAccount(escrowAuthority, mint),
        escrowAuthority,
        deriveNonceBitmapPda(forwarder, user, 1n)[0], // nonce 300 is in word 1
        TOKEN_PROGRAM_ID,
      ].map((k) => k.toBase58()),
    );
    expect(accounts.map((a) => a.isWritable)).toEqual([false, false, false, false, false, true, true, false, true, false]);
    expect(accounts.every((a) => !a.isSigner)).toBe(true);
  });

  it("unwrap segment is the forwarder layout", () => {
    const recipient = Keypair.generate().publicKey;
    const accounts = buildUnwrapForwarderAccounts(forwarder, recipient, mint);
    expect(accounts).toHaveLength(FORWARDER_UNWRAP_NUM_ACCOUNTS);

    const [escrowAuthority] = deriveForwarderEscrowAuthority(forwarder);
    expect(accounts.map((a) => a.pubkey.toBase58())).toEqual(
      [
        forwarder,
        deriveForwarderConfigPda(forwarder)[0],
        SYSVAR_INSTRUCTIONS_PUBKEY,
        deriveEventAuthorityPda(forwarder)[0],
        forwarder,
        tokenAccount(escrowAuthority, mint),
        tokenAccount(recipient, mint),
        escrowAuthority,
        TOKEN_PROGRAM_ID,
      ].map((k) => k.toBase58()),
    );
    expect(accounts.map((a) => a.isWritable)).toEqual([false, false, false, false, false, true, true, false, false]);
    expect(accounts.every((a) => !a.isSigner)).toBe(true);
  });

  it("migration segment is the emergency-migrating build's layout", () => {
    const adapter = Keypair.generate().publicKey;
    const entries = [1, 2].map((i) => ({
      nullifier: new Uint8Array(32).fill(i),
      commitmentTreeRoot: new Uint8Array(32).fill(10 + i),
    }));
    const accounts = buildMigrateForwarderAccounts(forwarder, adapter, entries);
    expect(accounts).toHaveLength(forwarderMigrateNumAccounts(2));
    expect(accounts.map((a) => a.pubkey.toBase58())).toEqual(
      [
        forwarder,
        deriveForwarderConfigPda(forwarder)[0],
        SYSVAR_INSTRUCTIONS_PUBKEY,
        deriveEventAuthorityPda(forwarder)[0],
        forwarder,
        derivePaStatePda(adapter)[0],
        deriveVulnerableLogicRefsPda(forwarder)[0],
        ...entries.flatMap((entry) => [
          deriveAdapterRootMarkerPda(adapter, entry.commitmentTreeRoot)[0],
          deriveAdapterNullifierPda(adapter, entry.nullifier)[0],
          deriveMigrationRecordPda(forwarder, entry.nullifier)[0],
        ]),
      ].map((k) => k.toBase58()),
    );
    // Only the migration records are written.
    expect(accounts.map((a) => a.isWritable)).toEqual([
      false, false, false, false, false, false, false, false, false, true, false, false, true,
    ]);
    expect(accounts.every((a) => !a.isSigner)).toBe(true);
  });

  it("init_migration_record instruction matches the emergency-migrating build's IDL", () => {
    const payer = Keypair.generate().publicKey;
    const nullifier = new Uint8Array(32).fill(5);
    const ix = initMigrationRecordIx(forwarder, payer, nullifier);
    expect(Array.from(ix.data.subarray(0, 8))).toEqual(
      emergencyMigratingIdl.instructions.find((i) => i.name === "init_migration_record")!.discriminator,
    );
    expect(Array.from(ix.data.subarray(8))).toEqual(Array.from(nullifier));
    expect(ix.keys.map((k) => [k.pubkey.toBase58(), k.isSigner, k.isWritable])).toEqual([
      [payer.toBase58(), true, true],
      [deriveMigrationRecordPda(forwarder, nullifier)[0].toBase58(), false, true],
      [SystemProgram.programId.toBase58(), false, false],
    ]);
  });

  it("nonce word index covers 256 nonces per word", () => {
    expect(nonceWordIndex(0n)).toBe(0n);
    expect(nonceWordIndex(255n)).toBe(0n);
    expect(nonceWordIndex(256n)).toBe(1n);
  });

  it("init_nonce_bitmap instruction matches the forwarder IDL", () => {
    const payer = Keypair.generate().publicKey;
    const user = Keypair.generate().publicKey;
    const ix = initNonceBitmapIx(forwarder, payer, user, 7n);

    expect(ix.programId.equals(forwarder)).toBe(true);
    // The IDL's discriminator, then the two args.
    expect(Array.from(ix.data.subarray(0, 8))).toEqual(
      forwarderIdl.instructions.find((i) => i.name === "init_nonce_bitmap")!.discriminator,
    );
    expect(Array.from(ix.data.subarray(8, 40))).toEqual(Array.from(user.toBytes()));
    expect(Array.from(ix.data.subarray(40))).toEqual([7, 0, 0, 0, 0, 0, 0, 0]);

    const [expectedBitmap] = deriveNonceBitmapPda(forwarder, user, 7n);
    expect(ix.keys.map((k) => [k.pubkey.toBase58(), k.isSigner, k.isWritable])).toEqual([
      [payer.toBase58(), true, true],
      [expectedBitmap.toBase58(), false, true],
      [SystemProgram.programId.toBase58(), false, false],
    ]);
  });
});
