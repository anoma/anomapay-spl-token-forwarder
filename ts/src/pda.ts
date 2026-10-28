// Program-derived-address helpers for the SPL Token Forwarder: its own from
// the seeds its IDLs declare, and the adapter's that a migration reads, whose
// seeds no forwarder IDL declares. They are pure functions: same inputs
// always produce the same PublicKey + bump.

import { PublicKey } from "@solana/web3.js";

import { emergencyMigratingIdl, idl, idlPda, idlSeed } from "./idl.js";

const u64Le = (value: bigint): Uint8Array => {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigUint64(0, value, true);
  return buf;
};

/** Derive the forwarder's global config PDA. Seed: `[CONFIG_SEED]`. */
export function deriveForwarderConfigPda(forwarderProgram: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([idlSeed(idl, "CONFIG_SEED")], forwarderProgram);
}

/**
 * Derive the forwarder's escrow authority. Seed: `[ESCROW_SEED]`.
 *
 * One PDA owns every mint's escrow: each escrow is the Associated Token Account
 * of this authority and the mint, as the EVM forwarder holds every token at its
 * own address. It is also the delegate users name in their SPL `Approve`
 * instruction before a wrap.
 */
export function deriveForwarderEscrowAuthority(forwarderProgram: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([idlSeed(idl, "ESCROW_SEED")], forwarderProgram);
}

/** Derive the forwarder's nonce bitmap PDA. Seed: `[NONCE_BITMAP_SEED, user, word_index_le]`, `word_index = nonce / NONCES_PER_WORD`. */
export function deriveNonceBitmapPda(
  forwarderProgram: PublicKey,
  user: PublicKey,
  wordIndex: bigint,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [idlSeed(idl, "NONCE_BITMAP_SEED"), user.toBuffer(), u64Le(wordIndex)],
    forwarderProgram,
  );
}

/** Derive the emergency-migrating build's vulnerable logic refs. Seed: `[VULNERABLE_LOGIC_REFS_SEED]`. */
export function deriveVulnerableLogicRefsPda(forwarderProgram: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [idlSeed(emergencyMigratingIdl, "VULNERABLE_LOGIC_REFS_SEED")],
    forwarderProgram,
  );
}

/** Derive the emergency-migrating build's record of the resource with `nullifier`. Seed: `[MIGRATION_RECORD_SEED, nullifier]`. */
export function deriveMigrationRecordPda(forwarderProgram: PublicKey, nullifier: Uint8Array): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [idlSeed(emergencyMigratingIdl, "MIGRATION_RECORD_SEED"), nullifier],
    forwarderProgram,
  );
}

/**
 * Derive the forwarder's event authority, from the seeds its IDL declares.
 *
 * Anchor's `#[event_cpi]` signs each event self-invocation with this PDA and
 * requires it, followed by the program's own address, after an instruction's
 * other named accounts: the forwarder's instructions that emit events (inside
 * the forwarder CPI segment for `forward_call`).
 */
export function deriveEventAuthorityPda(forwarderProgram: PublicKey): PublicKey {
  return idlPda(idl, "initialize", "event_authority", forwarderProgram);
}

/** Derive the forwarder's upgrade authority, through which only the program upgrades itself, from the seeds its IDL declares. */
export function deriveUpgradeAuthorityPda(forwarderProgram: PublicKey): PublicKey {
  return idlPda(idl, "initialize", "upgrade_authority", forwarderProgram);
}

/** The upgradeable loader, which owns every upgradeable program's ProgramData account. */
export const BPF_LOADER_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

/** Derive the upgradeable loader's ProgramData account of `program`, which `initialize` and `upgrade` take. */
export function deriveProgramDataAddress(program: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([program.toBuffer()], BPF_LOADER_UPGRADEABLE)[0];
}

/** Derive the adapter's state account, which a migration reads. Seed: `["pa_state"]`. */
export function derivePaStatePda(protocolAdapter: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([new TextEncoder().encode("pa_state")], protocolAdapter);
}

/**
 * Derive the adapter's marker of the historical root `root`, as the adapter's
 * client does, `paState` being its state account. Seed: `["root", pa_state, root]`.
 */
export function deriveAdapterRootMarkerPda(
  protocolAdapter: PublicKey,
  paState: PublicKey,
  root: Uint8Array,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([new TextEncoder().encode("root"), paState.toBuffer(), root], protocolAdapter);
}

/**
 * Derive the adapter's marker of the spent nullifier `nullifier`, as the
 * adapter's client does. Seed: `["nullifier", pa_state, nullifier]`.
 */
export function deriveAdapterNullifierPda(
  protocolAdapter: PublicKey,
  paState: PublicKey,
  nullifier: Uint8Array,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("nullifier"), paState.toBuffer(), nullifier],
    protocolAdapter,
  );
}
