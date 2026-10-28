import { describe, expect, it } from "vitest";

import { BN } from "@anchor-lang/core";
import { bytesToHex } from "@noble/hashes/utils";
import { PublicKey } from "@solana/web3.js";

import { decodeForwarderEventInstruction, EVENT_IX_TAG, EventDecodeError } from "./events.js";
import { emergencyMigratingIdl, idl } from "./idl.js";

const MINT = new Uint8Array(32).fill(1);
const USER = new Uint8Array(32).fill(2);
const OTHER = new Uint8Array(32).fill(3);
const ROOT = new Uint8Array(32).fill(4);

const u64Le = (v: bigint): Uint8Array => {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return b;
};

/** Event tag, the IDL's discriminator of `name`, then the body. */
const eventIx = (name: string, body: Uint8Array[]): Uint8Array =>
  Uint8Array.from(
    Buffer.concat([EVENT_IX_TAG, Uint8Array.from(emergencyMigratingIdl.events!.find((e) => e.name === name)!.discriminator), ...body]),
  );

/** A decoded value with keys as hex and integers as bigints, to compare against the expected fields. */
const plain = (value: unknown): unknown =>
  value instanceof PublicKey
    ? bytesToHex(value.toBytes())
    : BN.isBN(value)
      ? BigInt(value.toString())
      : value instanceof Uint8Array || Array.isArray(value)
        ? bytesToHex(Uint8Array.from(value as number[]))
        : value;

const hex = bytesToHex;

/** Every event either build emits, with its hand-built instruction data and its fields under their IDL names. */
const cases: [string, Uint8Array, Record<string, unknown>][] = [
  ["Wrapped", eventIx("Wrapped", [MINT, USER, u64Le(1_000_000n)]), { token: hex(MINT), from: hex(USER), amount: 1_000_000n }],
  [
    "Unwrapped",
    eventIx("Unwrapped", [MINT, USER, u64Le(2n ** 64n - 1n)]),
    { token: hex(MINT), to: hex(USER), amount: 2n ** 64n - 1n },
  ],
  ["Initialized", eventIx("Initialized", [u64Le(2n)]), { version: 2n }],
  [
    "OwnershipTransferred",
    eventIx("OwnershipTransferred", [USER, OTHER]),
    { previous_owner: hex(USER), new_owner: hex(OTHER) },
  ],
  ["Upgraded", eventIx("Upgraded", [ROOT]), { implementation_hash: hex(ROOT) }],
  [
    "LogicRefReplaced",
    eventIx("LogicRefReplaced", [USER, OTHER]),
    { previous_logic_ref: hex(USER), new_logic_ref: hex(OTHER) },
  ],
  [
    "Migrated",
    eventIx("Migrated", [MINT, OTHER, ROOT]),
    { token: hex(MINT), vulnerable_logic_ref: hex(OTHER), nullifier: hex(ROOT) },
  ],
  ["VulnerableLogicRefListed", eventIx("VulnerableLogicRefListed", [OTHER]), { logic_ref: hex(OTHER) }],
];

describe("decodeForwarderEventInstruction", () => {
  it("decodes every forwarder event from hand-built bytes", () => {
    for (const [name, data, expected] of cases) {
      const event = decodeForwarderEventInstruction(data);
      console.log(name, event);
      expect(event.name, name).toBe(name);
      expect(Object.fromEntries(Object.entries(event.data).map(([k, v]) => [k, plain(v)])), name).toEqual(expected);
    }
  });

  it("covers every event of the emergency-migrating build's IDL", () => {
    expect(cases.map(([name]) => name).sort()).toEqual(emergencyMigratingIdl.events!.map((e) => e.name).sort());
  });

  // The decoder reads with the emergency-migrating build's IDL: it declares
  // every production event alike, so it reads the production build's too.
  it("reads the production build's events as that build declares them", () => {
    for (const event of idl.events!) {
      const same = emergencyMigratingIdl.events!.find((e) => e.name === event.name);
      expect(same, event.name).toEqual(event);
      expect(
        emergencyMigratingIdl.types!.find((t) => t.name === event.name),
        event.name,
      ).toEqual(idl.types!.find((t) => t.name === event.name));
    }
  });

  it("rejects instruction data without the event tag", () => {
    const data = cases[0]![1].slice(8);
    expect(() => decodeForwarderEventInstruction(data)).toThrow(EventDecodeError);
    expect(() => decodeForwarderEventInstruction(data)).toThrow(/does not start with the event tag/);
  });

  it("rejects a discriminator no IDL event has", () => {
    const data = Uint8Array.from(Buffer.concat([EVENT_IX_TAG, new Uint8Array(8).fill(0xab), ROOT]));
    expect(() => decodeForwarderEventInstruction(data)).toThrow(EventDecodeError);
    expect(() => decodeForwarderEventInstruction(data)).toThrow("no IDL event has the discriminator abababababababab");
  });

  // The coder refuses a short last field that is a byte array; it reads a
  // short integer or key, which the length check then refuses.
  it("rejects a body that ends short", () => {
    const shortArray = (field: string) =>
      `the event body does not decode: RangeError: Invalid array for "${field}": length 32 requires 32 bytes, only 31 remaining bytes`;
    const expected: Record<string, string> = {
      Wrapped: "the Wrapped body ends 1 byte(s) short",
      Unwrapped: "the Unwrapped body ends 1 byte(s) short",
      Initialized: "the Initialized body ends 1 byte(s) short",
      OwnershipTransferred: "the OwnershipTransferred body ends 1 byte(s) short",
      Upgraded: shortArray("implementation_hash"),
      LogicRefReplaced: shortArray("new_logic_ref"),
      Migrated: shortArray("nullifier"),
      VulnerableLogicRefListed: shortArray("logic_ref"),
    };
    for (const [name, data] of cases) {
      const short = data.slice(0, -1);
      expect(() => decodeForwarderEventInstruction(short), name).toThrow(EventDecodeError);
      expect(() => decodeForwarderEventInstruction(short), name).toThrow(expected[name]!);
    }
  });

  it("rejects trailing bytes", () => {
    for (const [name, data] of cases) {
      const longer = new Uint8Array(data.length + 1);
      longer.set(data);
      expect(() => decodeForwarderEventInstruction(longer), name).toThrow(EventDecodeError);
      expect(() => decodeForwarderEventInstruction(longer), name).toThrow(`1 trailing byte(s) after the ${name} body`);
    }
  });
});
