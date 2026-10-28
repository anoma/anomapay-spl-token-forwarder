// The decoder of the one forwarder account the IDLs do not declare: the
// emergency-migrating build's `VulnerableLogicRefs`, which `reinitialize`
// grows in place and reads as a raw account, so no instruction takes it as a
// typed account and Anchor leaves it out of the IDL. The Rust crate's
// `decode_vulnerable_logic_refs` reads it alike. Every other account decodes
// with the IDL coders (`coder.ts`).

import { sha256 } from "@noble/hashes/sha2";

export class AccountDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AccountDecodeError";
  }
}

/** Anchor's discriminator of the `#[account]` `VulnerableLogicRefs`: `sha256("account:VulnerableLogicRefs")[..8]`. */
const VULNERABLE_LOGIC_REFS_DISCRIMINATOR = sha256(new TextEncoder().encode("account:VulnerableLogicRefs")).slice(0, 8);

/**
 * Decode the emergency-migrating build's `VulnerableLogicRefs` account: the
 * logic refs whose resources can emergency-migrate. As the program's own read,
 * bytes after the list are not read.
 */
export function decodeVulnerableLogicRefs(data: Uint8Array): Uint8Array[] {
  if (
    data.length < VULNERABLE_LOGIC_REFS_DISCRIMINATOR.length ||
    !VULNERABLE_LOGIC_REFS_DISCRIMINATOR.every((b, i) => data[i] === b)
  ) {
    throw new AccountDecodeError("the account is not a VulnerableLogicRefs");
  }
  const body = data.subarray(VULNERABLE_LOGIC_REFS_DISCRIMINATOR.length);
  if (body.length < 4) throw new AccountDecodeError("the account ends inside its list of logic refs");
  const count = new DataView(body.buffer, body.byteOffset, 4).getUint32(0, true);
  if (body.length < 4 + 32 * count) throw new AccountDecodeError("the account ends inside its list of logic refs");
  return Array.from({ length: count }, (_, i) => body.slice(4 + 32 * i, 4 + 32 * (i + 1)));
}
