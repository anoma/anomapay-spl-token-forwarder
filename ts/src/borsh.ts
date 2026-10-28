// The length of a value's Borsh encoding, from the IDL type it has, which
// Anchor's coder does not report: the event decoder refuses a body that ends
// short or has bytes after it, and the instances refuse arguments the
// coder's buffer cannot hold.

import type { Idl, IdlType } from "@anchor-lang/core";

/** The byte length of each fixed-size Borsh type. */
const FIXED_LENGTHS: Record<string, number> = {
  bool: 1,
  u8: 1,
  i8: 1,
  u16: 2,
  i16: 2,
  u32: 4,
  i32: 4,
  f32: 4,
  u64: 8,
  i64: 8,
  f64: 8,
  u128: 16,
  i128: 16,
  u256: 32,
  i256: 32,
  pubkey: 32,
};

/**
 * The length of the Borsh encoding of `value`, of the type `type` of `idl`, in
 * the form the coder reads and writes it: the bytes the coder read when it
 * decoded `value`, or the bytes it writes when it encodes it. The coder
 * reports neither.
 */
export function borshLength(idl: Idl, value: any, type: IdlType): number {
  if (typeof type === "string") {
    if (type === "bytes") return 4 + value.length;
    const fixed = FIXED_LENGTHS[type];
    if (fixed !== undefined) return fixed;
  } else if ("array" in type) {
    return value.reduce((n: number, v: unknown) => n + borshLength(idl, v, type.array[0]), 0);
  } else if ("vec" in type) {
    return value.reduce((n: number, v: unknown) => n + borshLength(idl, v, type.vec), 4);
  } else if ("defined" in type) {
    const def = idl.types?.find((t) => t.name === type.defined.name)?.type;
    if (def?.kind === "type") return borshLength(idl, value, def.alias);
    if (def?.kind === "struct" && def.fields !== undefined) {
      return def.fields.reduce((n: number, f) => {
        if (typeof f !== "object" || !("name" in f)) throw new Error(`${type.defined.name} has unnamed fields`);
        return n + borshLength(idl, value[f.name], f.type);
      }, 0);
    }
  }
  throw new Error(`no Borsh length for the IDL type ${JSON.stringify(type)}`);
}
