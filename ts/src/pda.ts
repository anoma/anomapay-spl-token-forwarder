// Program-derived-address helpers for the SPL Token Forwarder: its own from
// the seeds its IDLs declare, and the adapter's that a migration reads, whose
// seeds no forwarder IDL declares. They are pure functions: same inputs
// always produce the same address + bump.

import { findAssociatedTokenPda, TOKEN_PROGRAM_ADDRESS } from "@solana-program/token";
import {
  address,
  getAddressEncoder,
  getProgramDerivedAddress,
  getU64Encoder,
  type Address,
  type ProgramDerivedAddress,
} from "@solana/kit";

import { emergencyMigratingIdl, idl, idlPda, idlSeed } from "./idl.js";

const addressBytes = (a: Address) => getAddressEncoder().encode(a);

/** Derive the forwarder's global config PDA. Seed: `[CONFIG_SEED]`. */
export function deriveForwarderConfigPda(forwarderProgram: Address): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({ programAddress: forwarderProgram, seeds: [idlSeed(idl, "CONFIG_SEED")] });
}

/**
 * Derive the forwarder's escrow authority. Seed: `[ESCROW_SEED]`.
 *
 * One PDA owns every mint's escrow: each escrow is the Associated Token Account
 * of this authority and the mint, as the EVM forwarder holds every token at its
 * own address. It is also the delegate users name in their SPL `Approve`
 * instruction before a wrap.
 */
export function deriveForwarderEscrowAuthority(forwarderProgram: Address): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({ programAddress: forwarderProgram, seeds: [idlSeed(idl, "ESCROW_SEED")] });
}

/** Derive the forwarder's nonce bitmap PDA. Seed: `[NONCE_BITMAP_SEED, user, word_index_le]`, `word_index = nonce / NONCES_PER_WORD`. */
export function deriveNonceBitmapPda(
  forwarderProgram: Address,
  user: Address,
  wordIndex: bigint,
): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({
    programAddress: forwarderProgram,
    seeds: [idlSeed(idl, "NONCE_BITMAP_SEED"), addressBytes(user), getU64Encoder().encode(wordIndex)],
  });
}

/** Derive the emergency-migrating build's vulnerable logic refs. Seed: `[VULNERABLE_LOGIC_REFS_SEED]`. */
export function deriveVulnerableLogicRefsPda(forwarderProgram: Address): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({
    programAddress: forwarderProgram,
    seeds: [idlSeed(emergencyMigratingIdl, "VULNERABLE_LOGIC_REFS_SEED")],
  });
}

/** Derive the emergency-migrating build's record of the resource with `nullifier`. Seed: `[MIGRATION_RECORD_SEED, nullifier]`. */
export function deriveMigrationRecordPda(forwarderProgram: Address, nullifier: Uint8Array): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({
    programAddress: forwarderProgram,
    seeds: [idlSeed(emergencyMigratingIdl, "MIGRATION_RECORD_SEED"), nullifier],
  });
}

/**
 * Derive the forwarder's event authority, from the seeds its IDL declares.
 *
 * Anchor's `#[event_cpi]` signs each event self-invocation with this PDA and
 * requires it, followed by the program's own address, after an instruction's
 * other named accounts: the forwarder's instructions that emit events (inside
 * the forwarder CPI segment for `forward_call`).
 */
export function deriveEventAuthorityPda(forwarderProgram: Address): Promise<Address> {
  return idlPda(idl, "initialize", "event_authority", forwarderProgram);
}

/** Derive the forwarder's upgrade authority, through which only the program upgrades itself, from the seeds its IDL declares. */
export function deriveUpgradeAuthorityPda(forwarderProgram: Address): Promise<Address> {
  return idlPda(idl, "initialize", "upgrade_authority", forwarderProgram);
}

/** The upgradeable loader, which owns every upgradeable program's ProgramData account. */
export const BPF_LOADER_UPGRADEABLE = address("BPFLoaderUpgradeab1e11111111111111111111111");

/** The instructions sysvar, from which a wrap reads the user's ed25519 instruction. */
export const SYSVAR_INSTRUCTIONS_ADDRESS = address("Sysvar1nstructions1111111111111111111111111");

/** The clock sysvar, which the loader's upgrade takes. */
export const SYSVAR_CLOCK_ADDRESS = address("SysvarC1ock11111111111111111111111111111111");

/** The rent sysvar, which the loader's upgrade takes. */
export const SYSVAR_RENT_ADDRESS = address("SysvarRent111111111111111111111111111111111");

/** Derive the upgradeable loader's ProgramData account of `program`, which `initialize` and `upgrade` take. */
export async function deriveProgramDataAddress(program: Address): Promise<Address> {
  return (await getProgramDerivedAddress({ programAddress: BPF_LOADER_UPGRADEABLE, seeds: [addressBytes(program)] }))[0];
}

/** Derive the adapter's state account, which a migration reads. Seed: `["pa_state"]`. */
export function derivePaStatePda(protocolAdapter: Address): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({ programAddress: protocolAdapter, seeds: ["pa_state"] });
}

/**
 * Derive the adapter's marker of the historical root `root`, as the adapter's
 * client does, `paState` being its state account. Seed: `["root", pa_state, root]`.
 */
export function deriveAdapterRootMarkerPda(
  protocolAdapter: Address,
  paState: Address,
  root: Uint8Array,
): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({ programAddress: protocolAdapter, seeds: ["root", addressBytes(paState), root] });
}

/**
 * Derive the adapter's marker of the spent nullifier `nullifier`, as the
 * adapter's client does. Seed: `["nullifier", pa_state, nullifier]`.
 */
export function deriveAdapterNullifierPda(
  protocolAdapter: Address,
  paState: Address,
  nullifier: Uint8Array,
): Promise<ProgramDerivedAddress> {
  return getProgramDerivedAddress({
    programAddress: protocolAdapter,
    seeds: ["nullifier", addressBytes(paState), nullifier],
  });
}

/** `owner`'s associated token account for `mint` under the SPL Token program; the owner may be a PDA. */
export async function deriveTokenAccount(owner: Address, mint: Address): Promise<Address> {
  return (await findAssociatedTokenPda({ owner, mint, tokenProgram: TOKEN_PROGRAM_ADDRESS }))[0];
}
