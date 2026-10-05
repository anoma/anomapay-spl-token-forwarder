// Decoder for the SPL Token Forwarder's events.
//
// The forwarder emits every event as a self-invocation (Anchor `#[event_cpi]`):
// an inner instruction whose program is the forwarder and whose data is the
// 8-byte event tag, the event's 8-byte discriminator
// (`sha256("event:<Name>")[..8]`), and the Borsh-encoded body. Readers pass
// each forwarder-addressed inner instruction of a transaction through
// `decodeForwarderEventInstruction`. Events never appear in the program log.

import { bytesToHex } from "@noble/hashes/utils";
import {
  type FixedSizeDecoder,
  fixDecoderSize,
  getBytesDecoder,
  getStructDecoder,
  getU64Decoder,
  transformDecoder,
} from "@solana/codecs";

import { anchorDiscriminator } from "./discriminator.js";

/** `anchor_lang::event::EVENT_IX_TAG_LE`: the u64 `0x1d9acb512ea545e4` little-endian. */
export const EVENT_IX_TAG = new Uint8Array([0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d]);

/**
 * The forwarder escrowed `amount` of `tokenMint` from `from` for the wrap with
 * `nonce`, authorized for the action whose tree root is `actionTreeRoot`, as
 * the EVM forwarder's `Wrapped`.
 */
export interface WrappedEvent {
  name: "Wrapped";
  tokenMint: Uint8Array;
  from: Uint8Array;
  amount: bigint;
  nonce: bigint;
  actionTreeRoot: Uint8Array;
}

/** The forwarder released `amount` of `tokenMint` to `to`, as the EVM forwarder's `Unwrapped`. */
export interface UnwrappedEvent {
  name: "Unwrapped";
  tokenMint: Uint8Array;
  to: Uint8Array;
  amount: bigint;
}

/**
 * The emergency committee (`setBy`) named `emergencyCaller` as the forwarder's
 * emergency caller, as the EVM V1 forwarder's `EmergencyCallerSet`.
 */
export interface EmergencyCallerSetEvent {
  name: "EmergencyCallerSet";
  emergencyCaller: Uint8Array;
  setBy: Uint8Array;
}

/** The emergency caller (`caller`) moved `amount` of `tokenMint` from escrow to `to`. */
export interface EmergencyWithdrawEvent {
  name: "EmergencyWithdraw";
  tokenMint: Uint8Array;
  to: Uint8Array;
  amount: bigint;
  caller: Uint8Array;
}

/**
 * `initialize` or `reinitialize` set the forwarder's configuration to
 * `version`, as OpenZeppelin Initializable's `Initialized(version)`.
 */
export interface InitializedEvent {
  name: "Initialized";
  version: bigint;
}

/**
 * The ownership moved from `previousOwner` to `newOwner`, as OpenZeppelin
 * Ownable's `OwnershipTransferred`; the zero key stands for no owner (the
 * previous owner at `initialize`, the new owner once renounced).
 */
export interface OwnershipTransferredEvent {
  name: "OwnershipTransferred";
  previousOwner: Uint8Array;
  newOwner: Uint8Array;
}

/**
 * The owner upgraded the forwarder to the code whose executable hash is
 * `executableHash` (sha256 of the code without trailing zero bytes, what
 * `solana-verify get-program-hash` reports), as ERC1967's `Upgraded`.
 */
export interface UpgradedEvent {
  name: "Upgraded";
  executableHash: Uint8Array;
}

export type ForwarderEvent =
  | WrappedEvent
  | UnwrappedEvent
  | EmergencyCallerSetEvent
  | EmergencyWithdrawEvent
  | InitializedEvent
  | OwnershipTransferredEvent
  | UpgradedEvent;

export class EventDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EventDecodeError";
  }
}

/** A Borsh `[u8; 32]`. */
const bytes32 = transformDecoder(fixDecoderSize(getBytesDecoder(), 32), (b) => Uint8Array.from(b));
const u64 = getU64Decoder();

/** A body field: its name in the TypeScript event, and its Borsh decoder. */
type BodyField = readonly [string, FixedSizeDecoder<Uint8Array | bigint>];

/**
 * The reader of event `name`'s body, whose Borsh fields are `fields` in
 * order. A body that ends inside a field names the field (by its name in the
 * program, snake_case); bytes past the last field are refused.
 */
function bodyReader<N extends ForwarderEvent["name"], const F extends readonly BodyField[]>(name: N, fields: F) {
  const decoder = getStructDecoder(fields);
  return (body: Uint8Array) => {
    let end = 0;
    for (const [field, fieldDecoder] of fields) {
      end += fieldDecoder.fixedSize;
      if (body.length < end) {
        const programName = field.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
        throw new EventDecodeError(`event truncated while reading ${programName}`);
      }
    }
    if (body.length > end) {
      throw new EventDecodeError(`${body.length - end} trailing byte(s) after the event body`);
    }
    return { name, ...decoder.decode(body) };
  };
}

const BODY_READERS: { [N in ForwarderEvent["name"]]: (body: Uint8Array) => Extract<ForwarderEvent, { name: N }> } = {
  Wrapped: bodyReader("Wrapped", [
    ["tokenMint", bytes32],
    ["from", bytes32],
    ["amount", u64],
    ["nonce", u64],
    ["actionTreeRoot", bytes32],
  ]),
  Unwrapped: bodyReader("Unwrapped", [
    ["tokenMint", bytes32],
    ["to", bytes32],
    ["amount", u64],
  ]),
  EmergencyCallerSet: bodyReader("EmergencyCallerSet", [
    ["emergencyCaller", bytes32],
    ["setBy", bytes32],
  ]),
  EmergencyWithdraw: bodyReader("EmergencyWithdraw", [
    ["tokenMint", bytes32],
    ["to", bytes32],
    ["amount", u64],
    ["caller", bytes32],
  ]),
  Initialized: bodyReader("Initialized", [["version", u64]]),
  OwnershipTransferred: bodyReader("OwnershipTransferred", [
    ["previousOwner", bytes32],
    ["newOwner", bytes32],
  ]),
  Upgraded: bodyReader("Upgraded", [["executableHash", bytes32]]),
};

/** Each event's body reader, by the hex of its discriminator. */
const READERS_BY_DISCRIMINATOR = new Map<string, (body: Uint8Array) => ForwarderEvent>(
  Object.entries(BODY_READERS).map(([name, read]) => [bytesToHex(anchorDiscriminator("event", name)), read]),
);

const DISCRIMINATOR_END = EVENT_IX_TAG.length + 8;

/** Decode the instruction data of one SPL Token Forwarder event self-invocation. */
export function decodeForwarderEventInstruction(data: Uint8Array): ForwarderEvent {
  if (data.length < EVENT_IX_TAG.length || bytesToHex(data.subarray(0, EVENT_IX_TAG.length)) !== bytesToHex(EVENT_IX_TAG)) {
    throw new EventDecodeError("instruction data does not start with the Anchor event tag");
  }
  if (data.length < DISCRIMINATOR_END) {
    throw new EventDecodeError("event truncated while reading event discriminator");
  }
  const discriminator = bytesToHex(data.subarray(EVENT_IX_TAG.length, DISCRIMINATOR_END));
  const read = READERS_BY_DISCRIMINATOR.get(discriminator);
  if (read === undefined) {
    throw new EventDecodeError(`unknown event discriminator ${discriminator}`);
  }
  return read(data.subarray(DISCRIMINATOR_END));
}
