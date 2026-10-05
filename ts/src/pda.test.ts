import { describe, expect, it } from "vitest";
import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import { Keypair, PublicKey } from "@solana/web3.js";

import forwarderIdl from "../../crates/client/idl/spl_token_forwarder.json";
import {
  deriveAssociatedTokenAddress,
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
} from "./pda.js";
import { FORWARDER_PROGRAM_ID } from "./programIds.js";

/** The IDL's account `name` of instruction `instruction`. */
const idlAccount = (instruction: string, name: string) =>
  forwarderIdl.instructions.find((i) => i.name === instruction)!.accounts.find((a) => a.name === name)! as {
    address?: string;
    pda?: { seeds: { kind: string; value: number[] }[] };
  };

describe("PDA derivation", () => {
  it("forwarder config PDA is the address the IDL resolves", () => {
    const [config] = deriveForwarderConfigPda(FORWARDER_PROGRAM_ID);
    expect(config.toBase58()).toBe(idlAccount("close_config", "config").address);
  });

  it("forwarder event authority PDA is the one the IDL's seeds derive", () => {
    const seeds = idlAccount("initialize", "event_authority").pda!.seeds.map((s) => Uint8Array.from(s.value));
    const [expected] = PublicKey.findProgramAddressSync(seeds, FORWARDER_PROGRAM_ID);
    expect(deriveEventAuthorityPda(FORWARDER_PROGRAM_ID)[0].equals(expected)).toBe(true);
  });

  it("forwarder escrow authority matches the one the adapter repo derives", () => {
    // Independent pin: the adapter repo's deriveEscrowAuthority (seed "escrow")
    // for the V2 forwarder; the Rust crate pins the same value.
    const [authority, bump] = deriveForwarderEscrowAuthority(FORWARDER_PROGRAM_ID);
    expect(authority.toBase58()).toBe("G78SQtzYuo4YKDEECzh25rckXeJFjLMXy44iWKKG5rDG");
    expect(bump).toBe(255);
  });

  it("ATA derivation accepts a PDA owner, which the forwarder's escrow authority is", () => {
    const mint = Keypair.generate().publicKey;
    const [escrowAuthority] = deriveForwarderEscrowAuthority(FORWARDER_PROGRAM_ID);
    expect(PublicKey.isOnCurve(escrowAuthority.toBytes())).toBe(false);
    expect(
      deriveAssociatedTokenAddress(escrowAuthority, mint).equals(getAssociatedTokenAddressSync(mint, escrowAuthority, true)),
    ).toBe(true);
  });

  it("ATA derivation agrees with the SPL token library, owner then mint", () => {
    const owner = Keypair.generate().publicKey;
    const mint = Keypair.generate().publicKey;
    const expected = getAssociatedTokenAddressSync(mint, owner);
    expect(deriveAssociatedTokenAddress(owner, mint).equals(expected)).toBe(true);
  });
});
