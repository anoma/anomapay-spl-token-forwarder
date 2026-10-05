import { describe, expect, it } from "vitest";
import { PublicKey } from "@solana/web3.js";

import forwarderIdl from "../../crates/client/idl/spl_token_forwarder.json";
import {
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

  it("forwarder escrow authority is the devnet forwarder's", () => {
    // Independent pin: the devnet forwarder's escrow authority, as
    // docs/DEVNET_DEPLOYMENT.md records it; the Rust crate pins the same value.
    const [authority, bump] = deriveForwarderEscrowAuthority(FORWARDER_PROGRAM_ID);
    expect(authority.toBase58()).toBe("G78SQtzYuo4YKDEECzh25rckXeJFjLMXy44iWKKG5rDG");
    expect(bump).toBe(255);
  });
});
