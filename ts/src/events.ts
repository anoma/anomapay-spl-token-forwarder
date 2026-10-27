// Decoder for the SPL Token Forwarder's events.
//
// The forwarder emits every event as a self-invocation (Anchor `#[event_cpi]`):
// an inner instruction whose program is the forwarder and whose data is the
// 8-byte event tag, the event's 8-byte discriminator
// (`sha256("event:" ++ signature)[..8]`, the signature being the event's name
// and its fields' IDL types in order, as `Wrapped(pubkey,pubkey,u64)`), and
// the Borsh-encoded body. Readers pass
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

/** The forwarder escrowed `amount` of the token whose mint is `token` from `from`, as the EVM forwarder's `Wrapped`. */
export interface WrappedEvent {
  name: "Wrapped";
  token: Uint8Array;
  from: Uint8Array;
  amount: bigint;
}

/** The forwarder released `amount` of the token whose mint is `token` to `to`, as the EVM forwarder's `Unwrapped`. */
export interface UnwrappedEvent {
  name: "Unwrapped";
  token: Uint8Array;
  to: Uint8Array;
  amount: bigint;
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
 * `implementationHash` (sha256 of the code without trailing zero bytes, what
 * `solana-verify get-program-hash` reports), as ERC1967's `Upgraded`.
 */
export interface UpgradedEvent {
  name: "Upgraded";
  implementationHash: Uint8Array;
}

/**
 * The emergency-migrating build's `reinitialize` replaced the logic ref
 * `previousLogicRef` with `newLogicRef`, as the EVM draft's `LogicRefReplaced`.
 */
export interface LogicRefReplacedEvent {
  name: "LogicRefReplaced";
  previousLogicRef: Uint8Array;
  newLogicRef: Uint8Array;
}

/**
 * The emergency-migrating build migrated the resource with `nullifier` and the
 * logic ref `vulnerableLogicRef`, whose token's mint is `token`, as the EVM
 * draft's `Migrated`. No tokens moved: an indexer adding up `Wrapped` and
 * `Unwrapped` amounts must not count it.
 */
export interface MigratedEvent {
  name: "Migrated";
  token: Uint8Array;
  vulnerableLogicRef: Uint8Array;
  nullifier: Uint8Array;
}

/**
 * The emergency-migrating build's `reinitialize` listed `logicRef` as
 * vulnerable, as the EVM draft's `VulnerableLogicRefListed`.
 */
export interface VulnerableLogicRefListedEvent {
  name: "VulnerableLogicRefListed";
  logicRef: Uint8Array;
}

/** One SPL Token Forwarder event, of either build. */
export type ForwarderEvent =
  | WrappedEvent
  | UnwrappedEvent
  | InitializedEvent
  | OwnershipTransferredEvent
  | UpgradedEvent
  | LogicRefReplacedEvent
  | MigratedEvent
  | VulnerableLogicRefListedEvent;

export class EventDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EventDecodeError";
  }
}

/** The Borsh decoder of a field type, with the type's name in the IDL. */
type FieldDecoder = FixedSizeDecoder<Uint8Array | bigint> & { idlType: string };

/** A Borsh 32-byte field. */
const bytes32 = transformDecoder(fixDecoderSize(getBytesDecoder(), 32), (b) => Uint8Array.from(b));
const pubkey = { ...bytes32, idlType: "pubkey" };
const array32 = { ...bytes32, idlType: "[u8;32]" };
const u64 = { ...getU64Decoder(), idlType: "u64" };

/** A body field: its name in the TypeScript event, and its decoder. */
type BodyField = readonly [string, FieldDecoder];

/** An event body's reader, and the event's signature. */
type BodyReader<E> = ((body: Uint8Array) => E) & { signature: string };

/**
 * The reader of event `name`'s body, whose Borsh fields are `fields` in
 * order, with the event's signature: `name`, then the fields' IDL types in
 * parentheses. A body that ends inside a field names the field (by its name in
 * the program, snake_case); bytes past the last field are refused.
 */
function bodyReader<N extends ForwarderEvent["name"], const F extends readonly BodyField[]>(name: N, fields: F) {
  const decoder = getStructDecoder(fields);
  const signature = `${name}(${fields.map(([, field]) => field.idlType).join(",")})`;
  return Object.assign((body: Uint8Array) => {
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
  }, { signature });
}

const BODY_READERS: { [N in ForwarderEvent["name"]]: BodyReader<Extract<ForwarderEvent, { name: N }>> } = {
  Wrapped: bodyReader("Wrapped", [
    ["token", pubkey],
    ["from", pubkey],
    ["amount", u64],
  ]),
  Unwrapped: bodyReader("Unwrapped", [
    ["token", pubkey],
    ["to", pubkey],
    ["amount", u64],
  ]),
  Initialized: bodyReader("Initialized", [["version", u64]]),
  OwnershipTransferred: bodyReader("OwnershipTransferred", [
    ["previousOwner", pubkey],
    ["newOwner", pubkey],
  ]),
  Upgraded: bodyReader("Upgraded", [["implementationHash", array32]]),
  LogicRefReplaced: bodyReader("LogicRefReplaced", [
    ["previousLogicRef", array32],
    ["newLogicRef", array32],
  ]),
  Migrated: bodyReader("Migrated", [
    ["token", pubkey],
    ["vulnerableLogicRef", array32],
    ["nullifier", array32],
  ]),
  VulnerableLogicRefListed: bodyReader("VulnerableLogicRefListed", [["logicRef", array32]]),
};

/** Each event's body reader, by the hex of its discriminator: the hash of its signature. */
const READERS_BY_DISCRIMINATOR = new Map<string, (body: Uint8Array) => ForwarderEvent>(
  Object.values(BODY_READERS).map((read) => [bytesToHex(anchorDiscriminator("event", read.signature)), read]),
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
