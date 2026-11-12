import { describe, expect, it } from "vitest";
import { address, getProgramDerivedAddress, type Address } from "@solana/kit";

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
const FORWARDER = address(idl.address);

describe("PDA derivation", () => {
  // Each helper whose account either IDL names in some instruction, against
  // the address that IDL determines there: its fixed address, or its PDA of
  // constant seeds.
  it("each derived account is the address the IDLs determine", async () => {
    const helpers: Record<string, Address> = {
      config: (await deriveForwarderConfigPda(FORWARDER))[0],
      escrow_authority: (await deriveForwarderEscrowAuthority(FORWARDER))[0],
      event_authority: await deriveEventAuthorityPda(FORWARDER),
      upgrade_authority: await deriveUpgradeAuthorityPda(FORWARDER),
      program_data: await deriveProgramDataAddress(FORWARDER),
      vulnerable_logic_refs: (await deriveVulnerableLogicRefsPda(FORWARDER))[0],
    };
    const checked = new Set<string>();
    for (const of of [idl, emergencyMigratingIdl]) {
      for (const instruction of of.instructions) {
        for (const account of instruction.accounts) {
          const derived = helpers[account.name];
          if (derived === undefined || "accounts" in account) continue;
          let declared: Address | undefined;
          if (account.address !== undefined) declared = address(account.address);
          else if (account.pda !== undefined && account.pda.seeds.every((s) => s.kind === "const")) {
            const seeds = account.pda.seeds.map((s) => Uint8Array.from((s as { value: number[] }).value));
            declared = (await getProgramDerivedAddress({ programAddress: FORWARDER, seeds }))[0];
          }
          if (declared === undefined) continue;
          console.log(`${of.metadata.name} ${instruction.name}.${account.name}: ${declared}`);
          expect(derived, `${instruction.name}.${account.name}`).toBe(declared);
          checked.add(account.name);
        }
      }
    }
    expect([...checked].sort()).toEqual(Object.keys(helpers).sort());
  });

});
