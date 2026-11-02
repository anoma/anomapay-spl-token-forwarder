// The forwarder's IDLs, the production build's and the emergency-migrating
// build's: `crates/client/idls/`, the ones the Rust crate is generated from,
// which the package's `copy-data` script copies into `src/data/` so the
// package ships the IDLs it encodes and decodes with.

import type { Idl, IdlInstruction } from "@anchor-lang/core";
import { AccountRole, getProgramDerivedAddress, type AccountMeta, type Address } from "@solana/kit";

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

/** The instruction `name` that `of` declares. */
export function idlInstruction(of: Idl, name: string): IdlInstruction {
  const ix = of.instructions.find((i) => i.name === name);
  if (ix === undefined) throw new Error(`the IDL of ${of.metadata.name} declares no instruction ${name}`);
  return ix;
}

/** The role of an account with the IDL's `signer` and `writable` flags. */
function accountRole(signer: boolean, writable: boolean): AccountRole {
  if (signer) return writable ? AccountRole.WRITABLE_SIGNER : AccountRole.READONLY_SIGNER;
  return writable ? AccountRole.WRITABLE : AccountRole.READONLY;
}

/**
 * The accounts of the instruction `name` of `of`: `accounts` by their IDL
 * names, every one the IDL lists, in its order and with its flags.
 */
export function idlAccountMetas(of: Idl, name: string, accounts: Record<string, Address>): AccountMeta[] {
  const ix = idlInstruction(of, name);
  const names = ix.accounts.map((a) => a.name);
  const unknown = Object.keys(accounts).filter((n) => !names.includes(n));
  if (unknown.length > 0) throw new Error(`${name} takes no account ${unknown.join(", ")}`);
  return ix.accounts.map((a): AccountMeta => {
    if ("accounts" in a) throw new Error(`${name}: the account group ${a.name} is not supported`);
    const account = accounts[a.name];
    if (account === undefined) throw new Error(`${name} takes an account ${a.name} not given`);
    return { address: account, role: accountRole(a.signer ?? false, a.writable ?? false) };
  });
}
