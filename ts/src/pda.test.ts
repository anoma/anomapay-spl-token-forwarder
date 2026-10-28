import { describe, expect, it } from "vitest";
import { PublicKey } from "@solana/web3.js";

import { emergencyMigratingIdl, idl } from "./idl.js";
import {
  deriveEventAuthorityPda,
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveProgramDataAddress,
  deriveUpgradeAuthorityPda,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

/** The program both IDLs are built for. */
const FORWARDER = new PublicKey(idl.address);

describe("PDA derivation", () => {
  // Each helper whose account either IDL names in some instruction, against
  // the address that IDL determines there: its fixed address, or its PDA of
  // constant seeds.
  it("each derived account is the address the IDLs determine", () => {
    const helpers: Record<string, PublicKey> = {
      config: deriveForwarderConfigPda(FORWARDER)[0],
      escrow_authority: deriveForwarderEscrowAuthority(FORWARDER)[0],
      event_authority: deriveEventAuthorityPda(FORWARDER),
      upgrade_authority: deriveUpgradeAuthorityPda(FORWARDER),
      program_data: deriveProgramDataAddress(FORWARDER),
      vulnerable_logic_refs: deriveVulnerableLogicRefsPda(FORWARDER)[0],
    };
    const checked = new Set<string>();
    for (const of of [idl, emergencyMigratingIdl]) {
      for (const instruction of of.instructions) {
        for (const account of instruction.accounts) {
          const derived = helpers[account.name];
          if (derived === undefined || "accounts" in account) continue;
          let declared: PublicKey | undefined;
          if (account.address !== undefined) declared = new PublicKey(account.address);
          else if (account.pda !== undefined && account.pda.seeds.every((s) => s.kind === "const")) {
            const seeds = account.pda.seeds.map((s) => Uint8Array.from((s as { value: number[] }).value));
            declared = PublicKey.findProgramAddressSync(seeds, FORWARDER)[0];
          }
          if (declared === undefined) continue;
          console.log(`${of.metadata.name} ${instruction.name}.${account.name}: ${declared.toBase58()}`);
          expect(derived.toBase58(), `${instruction.name}.${account.name}`).toBe(declared.toBase58());
          checked.add(account.name);
        }
      }
    }
    expect([...checked].sort()).toEqual(Object.keys(helpers).sort());
  });

  it("forwarder escrow authority is the configured forwarder's", () => {
    // Independent pin: the configured forwarder's escrow authority, as
    // `solana find-program-derived-address <forwarder> string:escrow` derives
    // it; the Rust crate pins the same value.
    const [authority, bump] = deriveForwarderEscrowAuthority(FORWARDER);
    expect(authority.toBase58()).toBe("A3srA2cw6o11TCSyMpBbnpLVnu6RLe6eGZEFz3d2m8h1");
    expect(bump).toBe(255);
  });
});
