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
