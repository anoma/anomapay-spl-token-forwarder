// A legacy transaction compiled as kit compiles one, without kit's refusal of
// an invoked program that an instruction marks writable.
//
// The forwarder's `upgrade` (the instance's `upgradeIxs`) invokes the
// forwarder and passes its program account, writable, on to the upgradeable
// loader, which rewrites the account. The runtime keeps an invoked program's
// account writable when the upgradeable loader is in the message, but kit's
// message compiler refuses every invoked program an instruction marks
// writable (SOLANA_ERROR__TRANSACTION__INVOKED_PROGRAMS_MUST_NOT_BE_WRITABLE),
// so no kit-compiled transaction can carry the upgrade.

import {
  getAddressComparator,
  getCompiledTransactionMessageEncoder,
  isSignerRole,
  isWritableRole,
  mergeRoles,
  AccountRole,
  type Address,
  type BlockhashLifetimeConstraint,
  type Instruction,
  type Transaction,
  type TransactionMessageBytes,
  type TransactionWithBlockhashLifetime,
} from "@solana/kit";

/**
 * The legacy transaction of `instructions`, which `feePayer` pays for, under
 * `lifetime`, unsigned: the accounts in kit's order (the fee payer, then
 * signers before non-signers and writable before read-only accounts, each
 * group by address), each with the strongest role any instruction gives it.
 */
export function compileLegacyTransaction(
  feePayer: Address,
  instructions: readonly Instruction[],
  lifetime: BlockhashLifetimeConstraint,
): Transaction & TransactionWithBlockhashLifetime {
  const roles = new Map<Address, AccountRole>([[feePayer, AccountRole.WRITABLE_SIGNER]]);
  const merge = (address: Address, role: AccountRole) =>
    roles.set(address, mergeRoles(roles.get(address) ?? AccountRole.READONLY, role));
  for (const ix of instructions) {
    merge(ix.programAddress, AccountRole.READONLY);
    for (const account of ix.accounts ?? []) merge(account.address, account.role);
  }

  const compare = getAddressComparator();
  const rank = (role: AccountRole) => (isSignerRole(role) ? 0 : 2) + (isWritableRole(role) ? 0 : 1);
  const accounts = [...roles.keys()].sort((a, b) => {
    if (a === feePayer || b === feePayer) return a === feePayer ? -1 : 1;
    return rank(roles.get(a)!) - rank(roles.get(b)!) || compare(a, b);
  });
  const index = new Map(accounts.map((address, i) => [address, i]));
  const count = (f: (role: AccountRole) => boolean) => accounts.filter((a) => f(roles.get(a)!)).length;

  const numSignerAccounts = count(isSignerRole);
  const messageBytes = getCompiledTransactionMessageEncoder().encode({
    version: "legacy",
    header: {
      numSignerAccounts,
      numReadonlySignerAccounts: count((r) => r === AccountRole.READONLY_SIGNER),
      numReadonlyNonSignerAccounts: count((r) => r === AccountRole.READONLY),
    },
    staticAccounts: accounts,
    lifetimeToken: lifetime.blockhash,
    instructions: instructions.map((ix) => ({
      programAddressIndex: index.get(ix.programAddress)!,
      ...(ix.accounts ? { accountIndices: ix.accounts.map((a) => index.get(a.address)!) } : null),
      ...(ix.data ? { data: ix.data } : null),
    })),
  }) as TransactionMessageBytes;

  // The message's signers, none of which has signed yet, as kit's
  // compileTransaction gives them.
  return {
    messageBytes,
    signatures: Object.fromEntries(accounts.slice(0, numSignerAccounts).map((signer) => [signer, null])),
    lifetimeConstraint: lifetime,
  };
}
