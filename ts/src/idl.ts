// The forwarder's IDLs, the production build's and the emergency-migrating
// build's: `crates/client/idls/`, the ones the Rust crate is generated from,
// which the package's `copy-data` script copies into `src/data/` so the
// package ships the IDLs it encodes and decodes with.

import type { Idl } from "@anchor-lang/core";
import { getProgramDerivedAddress, type Address } from "@solana/kit";

import forwarderJson from "./data/spl_token_forwarder.json" with { type: "json" };
import emergencyMigratingJson from "./data/emergency_migrating_spl_token_forwarder.json" with { type: "json" };

/** The production build's IDL. */
export const idl = forwarderJson as Idl;

/** The emergency-migrating build's IDL. */
export const emergencyMigratingIdl = emergencyMigratingJson as Idl;

/** The value `of` declares for its constant `name`, as the IDL writes it. */
export function idlConstant(of: Idl, name: string): string {
  const constant = of.constants?.find((c) => c.name === name);
  if (constant === undefined) throw new Error(`the IDL of ${of.metadata.name} declares no constant ${name}`);
  return constant.value;
}

/** The bytes of the PDA seed constant `name` that `of` declares. */
export function idlSeed(of: Idl, name: string): Uint8Array {
  return Uint8Array.from(JSON.parse(idlConstant(of, name)) as number[]);
}

/**
 * The address of `program` that `of` declares for the account `account` of
 * its instruction `instruction`, from the IDL's seeds, which must all be
 * constants.
 */
export async function idlPda(of: Idl, instruction: string, account: string, program: Address): Promise<Address> {
  const declared = of.instructions.find((i) => i.name === instruction)?.accounts.find((a) => a.name === account);
  if (declared === undefined || "accounts" in declared || declared.pda === undefined) {
    throw new Error(`the IDL of ${of.metadata.name} declares no PDA ${instruction}.${account}`);
  }
  const seeds = declared.pda.seeds.map((seed) => {
    if (seed.kind !== "const") throw new Error(`${instruction}.${account}: the seed ${JSON.stringify(seed)} is no constant`);
    return Uint8Array.from(seed.value);
  });
  return (await getProgramDerivedAddress({ programAddress: program, seeds }))[0];
}
