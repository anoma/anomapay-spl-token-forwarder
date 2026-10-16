// Program-derived-address helpers for the SPL Token Forwarder.
//
// These mirror the seed schemas baked into the on-chain program. They are pure
// functions: same inputs always produce the same PublicKey + bump.

import { PublicKey } from "@solana/web3.js";

const u64Le = (value: bigint): Uint8Array => {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigUint64(0, value, true);
  return buf;
};

/** Derive the forwarder's global config PDA. */
export function deriveForwarderConfigPda(
  forwarderProgram: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("config")],
    forwarderProgram,
  );
}

/**
 * Derive the forwarder's escrow authority. Seed: `["escrow"]`.
 *
 * One PDA owns every mint's escrow: each escrow is the Associated Token Account
 * of this authority and the mint, as the EVM forwarder holds every token at its
 * own address. It is also the delegate users name in their SPL `Approve`
 * instruction before a wrap.
 */
export function deriveForwarderEscrowAuthority(forwarderProgram: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([new TextEncoder().encode("escrow")], forwarderProgram);
}

/** Derive the forwarder's nonce bitmap PDA. `word_index = nonce / 256`. */
export function deriveNonceBitmapPda(
  forwarderProgram: PublicKey,
  user: PublicKey,
  wordIndex: bigint,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [
      new TextEncoder().encode("nonce_bitmap"),
      user.toBuffer(),
      u64Le(wordIndex),
    ],
    forwarderProgram,
  );
}

/** Derive the emergency-migrating build's vulnerable logic refs. Seed: `["vulnerable_logic_refs"]`. */
export function deriveVulnerableLogicRefsPda(forwarderProgram: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([new TextEncoder().encode("vulnerable_logic_refs")], forwarderProgram);
}

/** Derive the emergency-migrating build's record of the resource with `nullifier`. Seed: `["migration_record", nullifier]`. */
export function deriveMigrationRecordPda(forwarderProgram: PublicKey, nullifier: Uint8Array): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([new TextEncoder().encode("migration_record"), nullifier], forwarderProgram);
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

/**
 * Derive a program's event authority PDA. Seed: `["__event_authority"]`.
 *
 * Anchor's `#[event_cpi]` signs each event self-invocation with this PDA and
 * requires it, followed by the program's own address, after an instruction's
 * other named accounts: the forwarder's instructions that emit events (inside
 * the forwarder CPI segment for `forward_call`).
 */
export function deriveEventAuthorityPda(program: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("__event_authority")],
    program,
  );
}
