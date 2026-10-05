// The program identifiers a wrap or unwrap names.

import { PublicKey } from "@solana/web3.js";

/**
 * SPL Token Forwarder program ID: the V2 forwarder's declared id, under which
 * this repository builds and deploys it.
 */
export const FORWARDER_PROGRAM_ID = new PublicKey(
  "BsfuXpxw8oCmZXnYijyQkUYNcCnuskFZbYizmWLnpSU7",
);

/**
 * Solana's native ed25519 signature-verification program. Used to carry verified
 * wrap-authorization signatures into the settle transaction.
 */
export const ED25519_PROGRAM_ID = new PublicKey(
  "Ed25519SigVerify111111111111111111111111111",
);

/**
 * Solana's `Instructions` sysvar — used by the forwarder to introspect the
 * ed25519-verify instruction at `ed25519_ix_index`.
 */
export const INSTRUCTIONS_SYSVAR_ID = new PublicKey(
  "Sysvar1nstructions1111111111111111111111111",
);
