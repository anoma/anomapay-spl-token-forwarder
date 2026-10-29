// Anchor's framing of the forwarder's CPI events.
//
// The forwarder emits every event as a self-invocation (Anchor
// `#[event_cpi]`): an inner instruction whose program is the forwarder and
// whose data is the 8-byte event tag, the event's discriminator, and its
// Borsh body. Readers take a transaction's inner instructions and pass each
// forwarder-addressed one through `decodeForwarderEventInstruction`, which
// reads the event with the emergency-migrating build's coder: that build
// declares the production build's events and its own. Events never appear in
// the program log.

import { borshLength } from "./borsh.js";
import { emergencyMigratingCoder } from "./coder.js";
import { emergencyMigratingIdl } from "./idl.js";

/** `anchor_lang::event::EVENT_IX_TAG_LE`: the u64 `0x1d9acb512ea545e4` little-endian. */
export const EVENT_IX_TAG = new Uint8Array([0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d]);

export class EventDecodeError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "EventDecodeError";
  }
}

const base64 = (bytes: Uint8Array): string => btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join(""));

/**
 * The event in `data`, the instruction data of one forwarder event
 * self-invocation: its IDL name and its fields under their IDL names, as the
 * coder decodes them. Refuses data without the event tag, a discriminator no
 * IDL event has, a body that does not decode (one that ends short), and
 * bytes after the body.
 */
export function decodeForwarderEventInstruction(data: Uint8Array): { name: string; data: any } {
  const tag = data.subarray(0, EVENT_IX_TAG.length);
  if (tag.length !== EVENT_IX_TAG.length || !tag.every((b, i) => b === EVENT_IX_TAG[i])) {
    throw new EventDecodeError("the instruction data does not start with the event tag");
  }
  const rest = data.subarray(EVENT_IX_TAG.length);
  let event: { name: string; data: any } | null;
  try {
    event = emergencyMigratingCoder.events.decode(base64(rest));
  } catch (e) {
    throw new EventDecodeError(`the event body does not decode: ${e}`, { cause: e });
  }
  if (event === null) {
    const discriminator = Array.from(rest.subarray(0, 8), (b) => b.toString(16).padStart(2, "0")).join("");
    throw new EventDecodeError(`no IDL event has the discriminator ${discriminator}`);
  }
  const name = event.name;
  const discriminator = emergencyMigratingIdl.events!.find((e) => e.name === name)!.discriminator;
  const body = rest.length - discriminator.length;
  const read = borshLength(emergencyMigratingIdl, event.data, { defined: { name } });
  // The coder refuses a body that ends short, but reads none of the bytes after one.
  if (read < body) throw new EventDecodeError(`${body - read} trailing byte(s) after the ${name} body`);
  return event;
}
