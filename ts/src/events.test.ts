import { describe, expect, it } from "vitest";

import { BN, BorshCoder, type Idl } from "@anchor-lang/core";
import { bytesToHex } from "@noble/hashes/utils";
import { PublicKey } from "@solana/web3.js";

import forwarderIdl from "../../crates/client/idl/spl_token_forwarder.json";
import { anchorDiscriminator } from "./discriminator.js";
import {
  decodeForwarderEventInstruction,
  EVENT_IX_TAG,
  EventDecodeError,
  type ForwarderEvent,
} from "./events.js";

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
    Buffer.concat([EVENT_IX_TAG, Uint8Array.from(forwarderIdl.events.find((e) => e.name === name)!.discriminator), ...body]),
  );

/** Every forwarder event with its hand-built instruction data and the value it must decode to. */
const forwarderCases: [ForwarderEvent["name"], Uint8Array, ForwarderEvent][] = [
  [
    "Wrapped",
    eventIx("Wrapped", [MINT, USER, u64Le(1_000_000n), u64Le(77n), ROOT]),
    { name: "Wrapped", tokenMint: MINT, from: USER, amount: 1_000_000n, nonce: 77n, actionTreeRoot: ROOT },
  ],
  [
    "Unwrapped",
    eventIx("Unwrapped", [MINT, USER, u64Le(2n ** 64n - 1n)]),
    { name: "Unwrapped", tokenMint: MINT, to: USER, amount: 2n ** 64n - 1n },
  ],
  [
    "EmergencyCallerSet",
    eventIx("EmergencyCallerSet", [USER, OTHER]),
    { name: "EmergencyCallerSet", emergencyCaller: USER, setBy: OTHER },
  ],
  [
    "EmergencyWithdraw",
    eventIx("EmergencyWithdraw", [MINT, USER, u64Le(42n), OTHER]),
    { name: "EmergencyWithdraw", tokenMint: MINT, to: USER, amount: 42n, caller: OTHER },
  ],
  ["Initialized", eventIx("Initialized", [u64Le(2n)]), { name: "Initialized", version: 2n }],
  [
    "OwnershipTransferred",
    eventIx("OwnershipTransferred", [USER, OTHER]),
    { name: "OwnershipTransferred", previousOwner: USER, newOwner: OTHER },
  ],
  ["Upgraded", eventIx("Upgraded", [ROOT]), { name: "Upgraded", executableHash: ROOT }],
];

describe("decodeForwarderEventInstruction", () => {
  it("decodes every forwarder event from hand-built bytes", () => {
    for (const [name, data, expected] of forwarderCases) {
      expect(decodeForwarderEventInstruction(data), name).toEqual(expected);
    }
  });

  // Anchor's coder reads each body from the IDL's field list, independently of
  // this decoder: a field order or width both the decoder and the hand-built
  // bytes got wrong would decode differently here.
  it("decodes every forwarder event as Anchor's IDL coder does", () => {
    const coder = new BorshCoder(forwarderIdl as Idl);
    const plain = (value: unknown): unknown =>
      value instanceof PublicKey
        ? bytesToHex(value.toBytes())
        : BN.isBN(value)
          ? BigInt(value.toString())
          : value instanceof Uint8Array || Array.isArray(value)
            ? bytesToHex(Uint8Array.from(value as number[]))
            : value;
    for (const [name, data] of forwarderCases) {
      const anchorEvent = coder.events.decode(Buffer.from(data.slice(8)).toString("base64"));
      expect(anchorEvent?.name, name).toBe(name);
      const { name: _, ...fields } = decodeForwarderEventInstruction(data);
      expect(
        Object.fromEntries(Object.entries(fields).map(([k, v]) => [k, plain(v)])),
        name,
      ).toEqual(
        Object.fromEntries(
          Object.entries(anchorEvent!.data).map(([k, v]) => [k.replace(/_(.)/g, (_m, c: string) => c.toUpperCase()), plain(v)]),
        ),
      );
    }
  });

  it("covers every IDL event", () => {
    expect(forwarderCases.map(([name]) => name).sort()).toEqual(forwarderIdl.events.map((e) => e.name).sort());
  });

  it("rejects instruction data without the event tag", () => {
    const data = forwarderCases[0]![1].slice(8);
    expect(() => decodeForwarderEventInstruction(data)).toThrow(EventDecodeError);
    expect(() => decodeForwarderEventInstruction(data)).toThrow(/does not start with the Anchor event tag/);
  });

  it("rejects instruction data that ends inside the discriminator", () => {
    const data = forwarderCases[0]![1].slice(0, 15);
    expect(() => decodeForwarderEventInstruction(data)).toThrow(EventDecodeError);
    expect(() => decodeForwarderEventInstruction(data)).toThrow("event truncated while reading event discriminator");
  });

  it("rejects an unknown discriminator", () => {
    // A PA event is not a forwarder event.
    const disc = anchorDiscriminator("event", "TransactionExecutedEvent");
    const data = Uint8Array.from(Buffer.concat([EVENT_IX_TAG, disc, ROOT]));
    expect(() => decodeForwarderEventInstruction(data)).toThrow(EventDecodeError);
    expect(() => decodeForwarderEventInstruction(data)).toThrow(`unknown event discriminator ${bytesToHex(disc)}`);
  });

  it("rejects a truncated body", () => {
    for (const [name, data] of forwarderCases) {
      expect(() => decodeForwarderEventInstruction(data.slice(0, -1)), name).toThrow(EventDecodeError);
      expect(() => decodeForwarderEventInstruction(data.slice(0, -1)), name).toThrow(/event truncated while reading/);
    }
    expect(() => decodeForwarderEventInstruction(forwarderCases[0]![1].slice(0, -1))).toThrow(
      "event truncated while reading action_tree_root",
    );
    // A body that ends before its second field names that field.
    expect(() => decodeForwarderEventInstruction(forwarderCases[0]![1].slice(0, 16 + 32 + 5))).toThrow(
      "event truncated while reading from",
    );
  });

  it("rejects trailing bytes", () => {
    for (const [name, data] of forwarderCases) {
      const longer = new Uint8Array(data.length + 1);
      longer.set(data);
      expect(() => decodeForwarderEventInstruction(longer), name).toThrow(EventDecodeError);
      expect(() => decodeForwarderEventInstruction(longer), name).toThrow("1 trailing byte(s) after the event body");
    }
  });
});
