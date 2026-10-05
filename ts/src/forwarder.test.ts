import { getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { Keypair, type PublicKey, SYSVAR_INSTRUCTIONS_PUBKEY, SystemProgram } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import forwarderIdl from "../../crates/client/idl/spl_token_forwarder.json";
import { FORWARDER_UNWRAP_NUM_ACCOUNTS, FORWARDER_WRAP_NUM_ACCOUNTS } from "./constants.js";
import {
  buildUnwrapForwarderAccounts,
  buildWrapForwarderAccounts,
  forwarderSettlementLookupKeys,
  initNonceBitmapIx,
  nonceWordIndex,
} from "./forwarder.js";
import {
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveNonceBitmapPda,
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

  it("the lookup keys are the segments' accounts no user or recipient changes", () => {
    const user = Keypair.generate().publicKey;
    const recipient = Keypair.generate().publicKey;
    const mints = [Keypair.generate().publicKey, Keypair.generate().publicKey];
    const lookup = forwarderSettlementLookupKeys(forwarder, mints).map((k) => k.toBase58());
    for (const m of mints) {
      const perSettlement = [tokenAccount(user, m), deriveNonceBitmapPda(forwarder, user, 0n)[0], tokenAccount(recipient, m)].map(
        (k) => k.toBase58(),
      );
      const segments = [...buildWrapForwarderAccounts(forwarder, user, m, 0n), ...buildUnwrapForwarderAccounts(forwarder, recipient, m)];
      for (const { pubkey } of segments) {
        const key = pubkey.toBase58();
        expect(lookup.includes(key), key).toBe(!perSettlement.includes(key));
      }
    }
    // The six accounts every call shares and one escrow per mint: every fixed
    // segment account once, nothing else.
    expect(lookup).toHaveLength(6 + mints.length);
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
