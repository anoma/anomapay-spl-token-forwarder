import { describe, expect, it } from "vitest";
import {
  address,
  appendTransactionMessageInstructions,
  blockhash,
  compileTransaction,
  createSolanaRpc,
  createTransactionMessage,
  getAddressDecoder,
  getCompiledTransactionMessageDecoder,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  type Instruction,
} from "@solana/kit";

import { idl } from "./idl.js";
import { SplTokenForwarder } from "./instance.js";
import { compileLegacyTransaction } from "./transaction.js";

const rpc = createSolanaRpc("http://127.0.0.1:1");
const key = (n: number) => getAddressDecoder().decode(new Uint8Array(32).fill(n));
const forwarder = SplTokenForwarder.at(rpc, address(idl.address));
const lifetime = { blockhash: blockhash(key(9)), lastValidBlockHeight: 7n };
const payer = key(1);

/** `instructions` compiled by kit's own compiler. */
const compiledByKit = (instructions: Instruction[]) =>
  compileTransaction(
    pipe(
      createTransactionMessage({ version: "legacy" }),
      (m) => setTransactionMessageFeePayer(payer, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(lifetime, m),
      (m) => appendTransactionMessageInstructions(instructions, m),
    ),
  );

describe("compileLegacyTransaction", () => {
  // Instructions with signers, writable and read-only accounts and a
  // read-only invoked program, which kit compiles.
  it("compiles what kit compiles to kit's bytes", async () => {
    const instructions = [
      await forwarder.initNonceBitmapIx(payer, key(2), 3n),
      await forwarder.transferOwnershipIx(key(4), key(5)),
      await forwarder.closeNonceBitmapsBatchIx(key(4), key(6), [key(7), key(8)]),
    ];
    const ours = compileLegacyTransaction(payer, instructions, lifetime);
    const kits = compiledByKit(instructions);
    console.log(`${ours.messageBytes.length} message bytes, signers ${Object.keys(ours.signatures)}`);
    expect(Array.from(ours.messageBytes)).toEqual(Array.from(kits.messageBytes));
    expect(ours.signatures).toEqual(kits.signatures);
    expect(ours.lifetimeConstraint).toEqual(lifetime);
  });

  // The upgrade passes the forwarder, which it invokes, writable to the
  // loader: kit refuses to compile it, and the account stays writable here.
  it("compiles the upgrade, whose invoked program is writable", async () => {
    const upgrade = await forwarder.upgradeIxs(key(4), key(5), payer);
    expect(() => compiledByKit(upgrade)).toThrow(/invoked and marked writable/);

    const message = getCompiledTransactionMessageDecoder().decode(
      compileLegacyTransaction(payer, upgrade, lifetime).messageBytes,
    );
    if (message.version !== "legacy") throw new Error(`a ${message.version} message, not a legacy one`);
    console.log(message.header, message.staticAccounts);
    const { numSignerAccounts, numReadonlyNonSignerAccounts } = message.header;
    const position = message.staticAccounts.indexOf(forwarder.programId);
    const writableNonSigners = message.staticAccounts.length - numReadonlyNonSignerAccounts;
    expect(position, "the forwarder is a writable non-signer").toBeGreaterThanOrEqual(numSignerAccounts);
    expect(position, "the forwarder is a writable non-signer").toBeLessThan(writableNonSigners);
    expect(message.instructions[1]!.programAddressIndex).toBe(position);
    expect(Object.keys(compileLegacyTransaction(payer, upgrade, lifetime).signatures).sort()).toEqual(
      [payer, key(4)].sort(),
    );
  });
});
