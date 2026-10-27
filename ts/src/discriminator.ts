import { sha256 } from "@noble/hashes/sha2";

/**
 * An Anchor discriminator: the first 8 bytes of `sha256("<namespace>:<name>")`,
 * which identifies an instruction (`global`, `name` its name) or an event
 * (`event`, `name` its signature: its name and its fields' IDL types) on chain.
 */
export function anchorDiscriminator(namespace: "global" | "event", name: string): Uint8Array {
  return sha256(new TextEncoder().encode(`${namespace}:${name}`)).slice(0, 8);
}
