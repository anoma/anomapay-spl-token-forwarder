import { describe, expect, it } from "vitest";
import type { Idl } from "@anchor-lang/core";
import { getSetComputeUnitLimitInstruction } from "@solana-program/compute-budget";
import {
  AccountRole,
  address,
  createSolanaRpc,
  generateKeyPairSigner,
  getAddressDecoder,
  getProgramDerivedAddress,
  getU64Encoder,
  isSignerRole,
  isWritableRole,
  type Address,
  type Instruction,
} from "@solana/kit";

import { UPGRADE_COMPUTE_UNIT_LIMIT } from "./constants.js";
import { emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
import {
  EmergencyMigratingSplTokenForwarderInstance,
  recordedAddress,
  returnedString,
  SplTokenForwarderInstance,
} from "./contract.js";
import { BindingsError } from "./error.js";
import {
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveProgramDataAddress,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

const DEVNET_GENESIS = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const MAINNET_GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

/** The error `f` throws, which must be a `BindingsError`. */
function bindingsError(f: () => unknown): BindingsError {
  try {
    f();
  } catch (error) {
    expect(error).toBeInstanceOf(BindingsError);
    return error as BindingsError;
  }
  throw new Error("no error was thrown");
}

describe("recordedAddress", () => {
  it("finds the recorded staging devnet forwarder at the IDL's address", () => {
    const found = recordedAddress("staging", DEVNET_GENESIS);
    console.log(found);
    expect(found).toBe(address(idl.address));
  });

  it("finds production recording nothing on mainnet-beta", () => {
    const e = bindingsError(() => recordedAddress("production", MAINNET_GENESIS));
    console.log(e.message);
    expect(e.detail).toEqual({ kind: "UnsupportedChain", cluster: "mainnet-beta" });
  });

  // A genesis hash of no known cluster, such as a local validator's, is
  // ChainIdUnknown in every environment.
  it("finds a genesis hash of no known cluster ChainIdUnknown", () => {
    for (const environment of ["staging", "production"] as const) {
      const e = bindingsError(() => recordedAddress(environment, "11111111111111111111111111111111"));
      console.log(`${environment}: ${e.message}`);
      expect(e.detail).toEqual({ kind: "ChainIdUnknown" });
    }
  });
});

// The builders read nothing: an RPC no server answers.
const rpc = createSolanaRpc("http://127.0.0.1:1");
const bytes = (n: number) => new Uint8Array(32).fill(n);
const key = (n: number): Address => getAddressDecoder().decode(bytes(n));
const u64Le = (v: bigint) => Uint8Array.from(getU64Encoder().encode(v));
const concat = (...parts: Uint8Array[]) => Uint8Array.from(Buffer.concat(parts));

/**
 * Checks `ix` against the instruction `name` of `of`: its data is the IDL's
 * discriminator then `args`, and each account has the IDL's flags, its fixed
 * address, or its PDA from the IDL's seeds when they are all constants, for
 * the program the IDL is built for.
 */
async function expectMatchesTheIdl(of: Idl, ix: Instruction, name: string, args: Uint8Array) {
  const spec = of.instructions.find((i) => i.name === name)!;
  expect(Array.from(ix.data!), `${name}: the discriminator, then the arguments`).toEqual([
    ...spec.discriminator,
    ...args,
  ]);
  const accounts = ix.accounts!;
  expect(accounts.length, `${name}: accounts`).toBeGreaterThanOrEqual(spec.accounts.length);
  for (const [i, account] of spec.accounts.entries()) {
    if ("accounts" in account) throw new Error(`${name}: the account group ${account.name}`);
    const meta = accounts[i]!;
    expect(isWritableRole(meta.role), `${name}.${account.name} writable`).toBe(account.writable ?? false);
    expect(isSignerRole(meta.role), `${name}.${account.name} signer`).toBe(account.signer ?? false);
    if (account.address !== undefined) {
      expect(meta.address, `${name}.${account.name} address`).toBe(account.address);
    }
    if (account.pda !== undefined && account.pda.seeds.every((s) => s.kind === "const")) {
      const seeds = account.pda.seeds.map((s) => Uint8Array.from((s as { value: number[] }).value));
      expect(meta.address, `${name}.${account.name} PDA`).toBe(
        (await getProgramDerivedAddress({ programAddress: ix.programAddress, seeds }))[0],
      );
    }
  }
}

/**
 * `of`'s instructions, but `forward_call`: only the adapter makes that call,
 * inside a settlement, with the accounts the segment builders
 * (`build*ForwarderAccounts`) give it.
 */
const sentInstructions = (of: Idl) =>
  of.instructions
    .map((i) => i.name)
    .filter((n) => n !== "forward_call")
    .sort();

/** The builders both builds' instances share, each with its arguments' expected bytes. */
async function sharedBuilders(
  f: SplTokenForwarderInstance | EmergencyMigratingSplTokenForwarderInstance,
): Promise<[string, Instruction, Uint8Array][]> {
  return [
    ["initialize", await f.initializeIx(key(1), key(2), bytes(3), key(4)), concat(bytes(2), bytes(3), bytes(4))],
    ["init_nonce_bitmap", await f.initNonceBitmapIx(key(1), key(2), 7n), concat(bytes(2), u64Le(7n))],
    ["transfer_ownership", await f.transferOwnershipIx(key(1), key(2)), bytes(2)],
    ["renounce_ownership", await f.renounceOwnershipIx(key(1)), new Uint8Array()],
    ["upgrade", (await f.upgradeIxs(key(1), key(2), key(3)))[1], new Uint8Array()],
    ["version", f.versionIx(), new Uint8Array()],
    ["close_escrow", await f.closeEscrowIx(key(1), key(2), key(3), key(4)), new Uint8Array()],
    ["close_nonce_bitmaps_batch", await f.closeNonceBitmapsBatchIx(key(1), key(2), [key(5), key(6)]), new Uint8Array()],
    ["close_config", await f.closeConfigIx(key(1), key(2)), new Uint8Array()],
  ];
}

const productionBuilders = async (f: SplTokenForwarderInstance): Promise<[string, Instruction, Uint8Array][]> => [
  ...(await sharedBuilders(f)),
  ["reinitialize", await f.reinitializeIx(key(1), bytes(3)), bytes(3)],
];

const emergencyMigratingBuilders = async (
  f: EmergencyMigratingSplTokenForwarderInstance,
): Promise<[string, Instruction, Uint8Array][]> => [
  ...(await sharedBuilders(f)),
  [
    "reinitialize",
    await f.reinitializeIx(key(1), key(2), bytes(3), [bytes(5), bytes(6)]),
    concat(bytes(3), Uint8Array.of(2, 0, 0, 0), bytes(5), bytes(6)),
  ],
  ["init_migration_record", await f.initMigrationRecordIx(key(1), bytes(5)), bytes(5)],
];

describe("the instances' builders", () => {
  const program = address(idl.address);

  for (const [build, of, built] of [
    ["production", idl, () => productionBuilders(new SplTokenForwarderInstance(program, rpc))],
    [
      "emergency-migrating",
      emergencyMigratingIdl,
      () => emergencyMigratingBuilders(new EmergencyMigratingSplTokenForwarderInstance(program, rpc)),
    ],
  ] as const) {
    it(`build every instruction a client sends the ${build} build, as its IDL declares it`, async () => {
      const names: string[] = [];
      for (const [name, ix, args] of await built()) {
        console.log(`${build} ${name}: ${ix.accounts!.length} accounts, ${ix.data!.length} bytes`);
        expect(ix.programAddress, name).toBe(program);
        await expectMatchesTheIdl(of, ix, name, args);
        names.push(name);
      }
      expect(names.sort()).toEqual(sentInstructions(of));
    });
  }

  // At another address, every account the program determines is that
  // program's: none is an address the IDL fixes for the program it is built
  // for.
  it("derive every account at the instance's own address", async () => {
    const other = (await generateKeyPairSigner()).address;
    const idlProgram = address(idl.address);
    const ofTheIdlProgram = new Set<Address>([
      idlProgram,
      (await deriveForwarderConfigPda(idlProgram))[0],
      (await deriveForwarderEscrowAuthority(idlProgram))[0],
      await deriveProgramDataAddress(idlProgram),
      (await deriveVulnerableLogicRefsPda(idlProgram))[0],
    ]);
    const otherConfig = (await deriveForwarderConfigPda(other))[0];
    const all = [
      ...(await productionBuilders(new SplTokenForwarderInstance(other, rpc))),
      ...(await emergencyMigratingBuilders(new EmergencyMigratingSplTokenForwarderInstance(other, rpc))),
    ];
    for (const [name, ix] of all) {
      expect(ix.programAddress, name).toBe(other);
      const accounts = ix.accounts!.map((a) => a.address);
      const leaked = accounts.filter((a) => ofTheIdlProgram.has(a));
      expect(leaked, `${name} names the IDL program's accounts`).toEqual([]);
      if (accounts.includes(otherConfig)) continue;
      expect(["init_nonce_bitmap", "init_migration_record", "version"], `${name} has no config`).toContain(name);
    }
  });

  it("put the upgrade's compute budget first", async () => {
    const [budget] = await new SplTokenForwarderInstance(program, rpc).upgradeIxs(key(1), key(2), key(3));
    expect(budget).toEqual(getSetComputeUnitLimitInstruction({ units: UPGRADE_COMPUTE_UNIT_LIMIT }));
  });

  it("put the bitmaps to close after the named accounts, writable", async () => {
    const ix = await new SplTokenForwarderInstance(program, rpc).closeNonceBitmapsBatchIx(key(1), key(2), [key(5), key(6)]);
    expect(ix.accounts!.slice(3)).toEqual([
      { address: key(5), role: AccountRole.WRITABLE },
      { address: key(6), role: AccountRole.WRITABLE },
    ]);
  });

  // The coder sizes each encoding by its value: 40 refs encode to 1324 bytes.
  it("encode a reinitialize of 40 vulnerable logic refs, more than 1000 bytes", async () => {
    const f = new EmergencyMigratingSplTokenForwarderInstance(program, rpc);
    const refs = Array.from({ length: 40 }, (_, i) => bytes(i + 1));
    const ix = await f.reinitializeIx(key(1), key(2), bytes(3), refs);
    const data = ix.data!;
    console.log(`reinitialize of 40 refs: ${data.length} bytes`);
    expect(data.length).toBe(8 + 32 + 4 + 40 * 32);
    expect(data.length).toBeGreaterThan(1000);
    expect(Array.from(data.subarray(8))).toEqual(Array.from(concat(bytes(3), Uint8Array.of(40, 0, 0, 0), ...refs)));
  });
});

describe("the instances' reads", () => {
  // Nothing listens on port 1, so every request fails: each read names what
  // it fetched, so every read is the one the read fetches its answer from.
  it("fail with RpcTransportError when the RPC cannot answer, naming what they read", async () => {
    const production = new SplTokenForwarderInstance(key(9), rpc);
    const emergency = new EmergencyMigratingSplTokenForwarderInstance(key(9), rpc);
    for (const [what, read] of [
      ["the forwarder's config", () => production.getLogicRef()],
      ["the forwarder's config", () => production.getProtocolAdapter()],
      ["the forwarder's config", () => production.owner()],
      ["the simulation of version", () => production.version(key(1))],
      ["the forwarder's config", () => emergency.getLogicRef()],
      ["the forwarder's config", () => emergency.getProtocolAdapter()],
      ["the forwarder's config", () => emergency.owner()],
      ["the simulation of version", () => emergency.version(key(1))],
      ["the vulnerable logic refs", () => emergency.isLogicRefVulnerable(bytes(1))],
      ["the migration record", () => emergency.isNullifierMigrated(bytes(1))],
    ] as const) {
      const e = await read().catch((error: unknown) => error);
      console.log(`${what}: ${String(e)}`);
      expect(e, what).toBeInstanceOf(BindingsError);
      expect(e, what).toMatchObject({ detail: { kind: "RpcTransportError", what: expect.stringMatching(new RegExp(`^${what}`)) } });
    }
  });
});

describe("the emergency-migrating instance", () => {
  it("names the IDL's legacy forwarder", () => {
    const declared = idlConstant(emergencyMigratingIdl, "LEGACY_FORWARDER");
    console.log(`LEGACY_FORWARDER: ${declared}`);
    expect(new EmergencyMigratingSplTokenForwarderInstance(key(9), rpc).legacyForwarder()).toBe(declared);
  });
});

describe("returnedString", () => {
  const program = key(9);
  const returnData = (from: Address, data: Uint8Array) => ({
    programId: from,
    data: [Buffer.from(data).toString("base64"), "base64"] as [string, "base64"],
  });
  // A Borsh String: its length as a little-endian u32, then the UTF-8.
  const encoded = concat(Uint8Array.of(5, 0, 0, 0), new TextEncoder().encode("0.4.0"));

  it("is the string the program returned", () => {
    expect(returnedString(program, returnData(program, encoded))).toBe("0.4.0");
  });

  it("refuses anything but one string the program returned", () => {
    for (const [what, data] of [
      ["nothing", null],
      ["another program's", returnData(key(8), encoded)],
      ["a byte short", returnData(program, encoded.subarray(0, encoded.length - 1))],
      ["a trailing byte", returnData(program, concat(encoded, Uint8Array.of(0)))],
      ["no bytes", returnData(program, new Uint8Array())],
      ["not UTF-8", returnData(program, Uint8Array.of(1, 0, 0, 0, 0xff))],
    ] as const) {
      const error = bindingsError(() => returnedString(program, data));
      console.log(`${what}: ${error.message}`);
      expect(error.detail.kind, what).toBe("Decode");
    }
  });
});
