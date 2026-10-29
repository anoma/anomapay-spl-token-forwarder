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
import type { DeploymentRecord } from "./deployments.js";
import { emergencyMigratingIdl, idl, idlConstant } from "./idl.js";
import {
  EmergencyMigratingSplTokenForwarder,
  InstanceError,
  recordedDeployment,
  returnedString,
  SplTokenForwarder,
} from "./instance.js";
import {
  deriveForwarderConfigPda,
  deriveForwarderEscrowAuthority,
  deriveProgramDataAddress,
  deriveVulnerableLogicRefsPda,
} from "./pda.js";

const DEVNET_GENESIS = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const MAINNET_GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

/** The error `f` throws, which must be an `InstanceError`. */
function instanceError(f: () => unknown): InstanceError {
  try {
    f();
  } catch (error) {
    expect(error).toBeInstanceOf(InstanceError);
    return error as InstanceError;
  }
  throw new Error("no error was thrown");
}

describe("recordedDeployment", () => {
  it("is no deployment on devnet while staging's program is not deployed yet", () => {
    const error = instanceError(() => recordedDeployment("staging", DEVNET_GENESIS));
    console.log(error.message);
    expect(error.detail).toEqual({ kind: "NoDeployment", chainId: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1" });
  });

  it("is no deployment where production records none", () => {
    expect(instanceError(() => recordedDeployment("production", MAINNET_GENESIS)).detail).toEqual({
      kind: "NoDeployment",
      chainId: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    });
  });

  // A cluster no environment records, such as a local validator, is no
  // deployment, under the chain id its genesis hash gives.
  it("is no deployment on an unrecorded cluster", () => {
    for (const environment of ["staging", "production"] as const) {
      expect(instanceError(() => recordedDeployment(environment, "11111111111111111111111111111111")).detail).toEqual({
        kind: "NoDeployment",
        chainId: "solana:11111111111111111111111111111111",
      });
    }
  });

  it("is the cluster's deployed entry", () => {
    const record: DeploymentRecord = {
      staging: [{ chainId: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1", programId: idl.address, deploymentSlot: 4242 }],
      production: [],
    };
    const found = recordedDeployment("staging", DEVNET_GENESIS, record);
    console.log(found);
    expect(found).toEqual({
      chainId: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
      programId: address(idl.address),
      deploymentSlot: 4242,
    });
    expect(() => recordedDeployment("production", DEVNET_GENESIS, record)).toThrow(InstanceError);
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
  f: SplTokenForwarder | EmergencyMigratingSplTokenForwarder,
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

const productionBuilders = async (f: SplTokenForwarder): Promise<[string, Instruction, Uint8Array][]> => [
  ...(await sharedBuilders(f)),
  ["reinitialize", await f.reinitializeIx(key(1), bytes(3)), bytes(3)],
];

const emergencyMigratingBuilders = async (
  f: EmergencyMigratingSplTokenForwarder,
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
    ["production", idl, () => productionBuilders(SplTokenForwarder.at(rpc, program))],
    [
      "emergency-migrating",
      emergencyMigratingIdl,
      () => emergencyMigratingBuilders(EmergencyMigratingSplTokenForwarder.at(rpc, program)),
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
      ...(await productionBuilders(SplTokenForwarder.at(rpc, other))),
      ...(await emergencyMigratingBuilders(EmergencyMigratingSplTokenForwarder.at(rpc, other))),
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
    const [budget] = await SplTokenForwarder.at(rpc, program).upgradeIxs(key(1), key(2), key(3));
    expect(budget).toEqual(getSetComputeUnitLimitInstruction({ units: UPGRADE_COMPUTE_UNIT_LIMIT }));
  });

  it("put the bitmaps to close after the named accounts, writable", async () => {
    const ix = await SplTokenForwarder.at(rpc, program).closeNonceBitmapsBatchIx(key(1), key(2), [key(5), key(6)]);
    expect(ix.accounts!.slice(3)).toEqual([
      { address: key(5), role: AccountRole.WRITABLE },
      { address: key(6), role: AccountRole.WRITABLE },
    ]);
  });

  // The coder sizes each encoding by its value: 40 refs encode to 1324 bytes.
  it("encode a reinitialize of 40 vulnerable logic refs, more than 1000 bytes", async () => {
    const f = EmergencyMigratingSplTokenForwarder.at(rpc, program);
    const refs = Array.from({ length: 40 }, (_, i) => bytes(i + 1));
    const ix = await f.reinitializeIx(key(1), key(2), bytes(3), refs);
    const data = ix.data!;
    console.log(`reinitialize of 40 refs: ${data.length} bytes`);
    expect(data.length).toBe(8 + 32 + 4 + 40 * 32);
    expect(data.length).toBeGreaterThan(1000);
    expect(Array.from(data.subarray(8))).toEqual(Array.from(concat(bytes(3), Uint8Array.of(40, 0, 0, 0), ...refs)));
  });
});

describe("the emergency-migrating instance", () => {
  it("names the IDL's legacy forwarder", () => {
    const declared = idlConstant(emergencyMigratingIdl, "LEGACY_FORWARDER");
    console.log(`LEGACY_FORWARDER: ${declared}`);
    expect(EmergencyMigratingSplTokenForwarder.at(rpc, key(9)).legacyForwarder()).toBe(declared);
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
      const error = instanceError(() => returnedString(program, data));
      console.log(`${what}: ${error.message}`);
      expect(error.detail.kind, what).toBe("Decode");
    }
  });
});
