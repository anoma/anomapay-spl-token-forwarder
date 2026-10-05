import paIdl from "../idl/protocol_adapter.json";

type IdlWithConstants = { metadata: { name: string }; constants: { name: string; value: string }[] };

// A program's `#[constant]`, as its IDL renders it: Rust's `{:?}` of the value.
function idlConstant(idl: IdlWithConstants, name: string): string {
  const constant = idl.constants.find((c) => c.name === name);
  if (!constant) {
    throw new Error(`The ${idl.metadata.name} IDL has no constant ${name}: rebuild the IDLs (anchor idl build).`);
  }
  return constant.value;
}

// A byte-string or byte-array constant, rendered as a list of numbers.
function idlBytes(idl: IdlWithConstants, name: string): Buffer {
  return Buffer.from(JSON.parse(idlConstant(idl, name)) as number[]);
}

// Protocol adapter (idl/protocol_adapter.json)
export const PA_STATE_SEED = idlBytes(paIdl, "PA_STATE_SEED");
/** Seed of a program's upgrade authority PDA, through which only the program upgrades itself. */
export const UPGRADE_AUTHORITY_SEED = idlBytes(paIdl, "UPGRADE_AUTHORITY_SEED");
