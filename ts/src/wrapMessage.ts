// Wrap authorization message: the 120-byte struct the user signs (via ed25519)
// to authorize a wrap. Replaces Permit2 / EIP-712 from the EVM stack.
//
// Layout (little-endian where multi-byte):
//   forwarder_id     : [u8; 32]
//   token_mint       : [u8; 32]
//   amount           : u64
//   nonce            : u64
//   deadline         : i64       // signed
//   action_tree_root : [u8; 32]
//
// Total: 120 bytes. The user signs `base64(sha256(layout))` as UTF-8 text —
// Phantom refuses raw binary input to signMessage because it could be a tx.

import { sha256 } from "@noble/hashes/sha2";

import { assertSplTokenAmount } from "./amount.js";

/** Byte length of the serialized wrap message. */
export const WRAP_MESSAGE_LEN = 120;

/** Inputs to construct a wrap authorization message. */
export interface WrapMessageInput {
  /** Forwarder program ID (32 bytes). */
  forwarderId: Uint8Array;
  /** SPL token mint (32 bytes). */
  tokenMint: Uint8Array;
  /** SPL token amount (u64). */
  amount: bigint;
  /** Per-user nonce (u64). */
  nonce: bigint;
  /** Unix timestamp in seconds (i64; allows negative sentinels). */
  deadline: bigint;
  /** ARM action tree root (32 bytes). */
  actionTreeRoot: Uint8Array;
}

const I64_MIN = -(1n << 63n);
const I64_MAX = (1n << 63n) - 1n;

const require32Bytes = (bytes: Uint8Array, label: string): Uint8Array => {
  if (bytes.length !== 32) {
    throw new Error(`${label} must be 32 bytes; got ${bytes.length}`);
  }
  return bytes;
};

/**
 * Build the canonical 120-byte little-endian wrap message.
 *
 * Validates `amount` and `nonce` (u64 range) and `deadline` (i64 range)
 * before serializing.
 */
export function buildWrapMessage(input: WrapMessageInput): Uint8Array<ArrayBuffer> {
  assertSplTokenAmount(input.amount, "Wrap amount");
  assertSplTokenAmount(input.nonce, "wrap nonce");
  if (input.deadline < I64_MIN || input.deadline > I64_MAX) {
    throw new RangeError(`wrap deadline must fit in i64; got ${input.deadline}`);
  }

  const forwarder = require32Bytes(input.forwarderId, "forwarderId");
  const mint = require32Bytes(input.tokenMint, "tokenMint");
  const root = require32Bytes(input.actionTreeRoot, "actionTreeRoot");

  const out = new Uint8Array(new ArrayBuffer(WRAP_MESSAGE_LEN));
  out.set(forwarder, 0);
  out.set(mint, 32);
  const view = new DataView(out.buffer);
  view.setBigUint64(64, input.amount, true);
  view.setBigUint64(72, input.nonce, true);
  view.setBigInt64(80, input.deadline, true);
  out.set(root, 88);
  return out;
}

/** SHA-256 of the serialized 120-byte layout. */
export function hashWrapMessage(input: WrapMessageInput): Uint8Array {
  return sha256(buildWrapMessage(input));
}

/**
 * Base64-encode the SHA-256 digest as UTF-8 text. This is the exact byte string
 * the wallet's `signMessage` must sign and the on-chain `Ed25519Program`
 * instruction must carry as its message bytes.
 */
export function base64WrapDigest(input: WrapMessageInput): string {
  const digest = hashWrapMessage(input);
  return btoa(String.fromCharCode(...digest));
}
