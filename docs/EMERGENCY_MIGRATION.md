# Emergency migration

The emergency-migrating build of the forwarder (the `emergency-migrating` Cargo feature, [`programs/spl-token-forwarder/src/emergency_migrating.rs`](../programs/spl-token-forwarder/src/emergency_migrating.rs)) keeps the forwarder usable after someone finds a flaw in a circuit version of the AnomaPay transfer resource that lets false proofs pass, for example in the circuit code or in the RISC Zero version that it uses. It is the port of the draft `EmergencyMigratingERC20Forwarder` of [anoma/anomapay-erc20-forwarder](https://github.com/anoma/anomapay-erc20-forwarder/tree/next) (`contracts/src/draft/`), whose `docs/emergency-migration.md` describes the same mechanism for the ERC20 forwarder. Like the EVM draft, no script deploys it.

This document lists the cases in which the forwarder moves to a new circuit version, says how the resources move in each case, and describes the build.

## Terms

- **Circuit version**: one build of the transfer resource's logic circuit, named by its logic ref (its verifying key).
- **Active version**: the circuit version the forwarder's config accepts.
- **Deprecated version**: a circuit version the kind tables list as an alias of the active one. Its resources convert into resources of the active version.
- **Vulnerable logic ref**: the logic ref of a circuit version with a flaw, which the adapter denies on both of its denylists and the forwarder lists.
- **Legacy-forwarder resource**: a resource with the cluster's legacy SPL token forwarder in its label, whose balances moved to this forwarder. No cluster has one: `LEGACY_FORWARDER_PROGRAM_ID` is the zero key in every `env/<cluster>.env`.
- **Soft migration**: a transaction that consumes resources of a deprecated version and creates resources of the active version, through the kind table.
- **Emergency migration**: a transaction that creates resources of the active version for resources with a vulnerable logic ref. The adapter does not consume the old resources; the forwarder records their nullifiers, and the tokens stay in escrow.

## Cases

The examples name circuit versions with letters, in release order. Each case starts from the same state:

- **A**: a deprecated version.
- **B**: the active version.
- **C**: the version that the forwarder moves to.
- **D**: the version after C, only in [A second incident](#a-second-incident).

### Voluntary upgrade

No circuit version has a flaw. C improves B. This is the rotation in [OPERATIONS.md](OPERATIONS.md) ("Rotating the logic ref"): the kind tables release C with B as its alias, the adapter's owner stores the new kind table, the forwarder's owner upgrades the forwarder to a build that raises `CONFIG_VERSION` and calls its `reinitialize` with the logic ref of C, and owners of resources of A and B soft-migrate them to C. It needs no emergency migration.

After an incident, every later build must keep the emergency migration, because the resources with a listed logic ref have no other way out. A voluntary upgrade then also uses an emergency-migrating build, whose `reinitialize` lists at least one vulnerable logic ref.

### Upgrade after a flaw

Each incident follows these steps:

1. The owner of the protocol adapter pauses it (the adapter's `pause`). The adapter then settles no transaction.
2. The owner of the adapter denies each vulnerable logic ref with `deny_logic_refs`, on both denylists (`consumed: true` and `consumed: false`).
3. The owner of the forwarder upgrades it in place to the emergency-migrating build, writing the build to a buffer and calling the forwarder's `upgrade` (no script builds or deploys it, as no script deploys the EVM draft), and calls its `reinitialize` with the logic ref of C and the vulnerable logic refs to list (see [Which resources can emergency-migrate](#which-resources-can-emergency-migrate)). The owner pays the rent of the list. C has no flaw and has the migrate call: only a circuit with the migrate call can start an emergency migration, so if the active version has none, the forwarder must move to a version that has one, even if the active version has no flaw.
4. The kind tables release C as the active version. They keep a vulnerable version listed as deprecated, but no transaction can use its rows, because the adapter denies its logic ref. The owner of the adapter stores the new kind table root. If B has no flaw, the owner deprecates its logic ref.
5. The owner of the adapter unpauses it.
6. Resource owners emergency-migrate their resources with a vulnerable logic ref, and soft-migrate their resources of a deprecated version.

`reinitialize` must list at least one logic ref. It checks that the adapter denies each listed logic ref on both denylists, and the logic ref of C on neither. It emits `VulnerableLogicRefListed` for each listed ref, `LogicRefReplaced` when the logic ref changes, and `Initialized` with the build's `CONFIG_VERSION`, one more than the production build's.

The cases differ in the circuit versions with the flaw:

| Flaw in | C | The adapter denies | `reinitialize` lists | Soft migration | Emergency migration |
| --- | --- | --- | --- | --- | --- |
| B, the active version | fixes B | B | B | A | B |
| A, a deprecated version | adds the migrate call to B | A | A | B | A |
| A and B, for example through a RISC Zero version that both use | fixes A and B | A and B | A and B | none | A and B |

If more deprecated versions have the flaw, the adapter denies each of them, and `reinitialize` lists them too.

### A second incident

After an incident, C is active and has the migrate call. `reinitialize` runs only once per build, so each later incident needs an upgrade to a new emergency-migrating build whose `CONFIG_VERSION` is one more again. If a later flaw is in C, the owner follows the steps above with D, which fixes C. If a later flaw is only in a deprecated version, for example A, C stays active: the owner upgrades the forwarder and calls `reinitialize` with the logic ref of C and the list with A. `reinitialize` can keep the logic ref. The logic refs of the first incident stay listed, so their resources can still emergency-migrate to the active version.

### Flaws that the forwarder cannot handle

- **A flaw in the RISC Zero verifier or in the compliance circuit.** It lets false proofs pass for every circuit. The owner of the adapter upgrades the adapter to a build without the flaw. An emergency migration does not help. If the flaw is in a RISC Zero version that circuit versions of the forwarder also use, the owner also follows the steps above.
- **A flaw in the circuit of another application.** The owner of that application handles it. This forwarder must not list its logic ref (see [Which resources can emergency-migrate](#which-resources-can-emergency-migrate)).

## Emergency migration transactions

An emergency migration transaction consumes one ephemeral resource of the active version, the trigger, and creates resources of the active version for the same total quantity. For each migrated resource, the trigger's logic proves that the commitment tree at a given root contains the resource's commitment, and computes the resource's nullifier. One signature of the resource owner over the action tree root authorizes all resources of the batch, so all of them must have the same authorization key.

The trigger's external call carries the forwarder instruction data `OP_MIGRATE` (2) followed by the Borsh encoding of

```rust
struct MigrateInput {
    token_mint: Pubkey,         // The mint in the label of the migrated resources.
    quantity: u64,              // The batch's total quantity, which the trigger's logic checks.
    entries: Vec<MigrateEntry>,
}

struct MigrateEntry {
    nullifier: [u8; 32],             // The nullifier of the resource.
    commitment_tree_root: [u8; 32],  // The commitment tree root that the resource is proven against.
    vulnerable_logic_ref: [u8; 32],  // The logic ref of the resource.
    forwarder: Pubkey,               // The forwarder in the resource label.
}
```

which the client crate encodes (`encode_migrate_forwarder_input`). The call's account segment is the forwarder's segment head, the adapter's state account and the forwarder's vulnerable logic refs, then for each entry the adapter's marker of its root, the adapter's marker of its nullifier and the entry's migration record: `7 + 3 × entries` accounts (`forwarder_migrate_num_accounts`), which the trigger's call commits. The adapter forwards no signer to a forwarder, so the forwarder cannot create an account during the call: like a wrap's nonce bitmap, each entry's migration record is created beforehand by the permissionless `init_migration_record`, which the client's `settlement_calls` puts in the settlement transaction.

Before it reads the first entry, the forwarder checks that both denylists of the protocol adapter still contain every vulnerable logic ref that it lists. Then, for each entry, it checks that:

- the forwarder in the resource label is this forwarder or the legacy forwarder of the cluster,
- the forwarder lists the logic ref as vulnerable,
- the root is a historical root of the adapter, as the adapter's own check for a consumed resource decides it (`protocol_adapter::root::is_root_valid`),
- the adapter's marker of the nullifier, at its derived address, is not the adapter's: the adapter did not consume the resource,
- the forwarder did not migrate the resource before.

The forwarder records each nullifier before it reads the next entry, so a batch that contains one resource two times fails. It emits `Migrated` for each entry. An emergency migration moves no tokens, so an indexer that adds up the `Wrapped` and `Unwrapped` amounts must not count it as a deposit.

## Why any historical root works

The root check ties the resource to the adapter's commitment tree. Without it, a prover could build a Merkle tree around a resource that never existed. Any historical root of the adapter is enough, and the forwarder stores no root of its own:

- After the denial, no transaction can create a resource with the vulnerable logic ref, so no root contains one that an earlier root does not.
- The nullifier checks stop a second use of a resource, whichever root its proof uses.

The backend can therefore prove against the adapter's latest root.

## Which resources can emergency-migrate

The forwarder keeps its address when it is upgraded, so the label `sha256(forwarder, mint)` of its resources is the same for every circuit version. The logic ref is different, and it is part of the resource kind.

The forwarder must emergency-migrate only resources whose tokens it holds, not every resource whose logic ref the adapter denies. So an entry must meet two conditions:

- **The label names this forwarder or the legacy forwarder of the cluster.** Anyone can deploy a program that acts as a forwarder, and a resource with that program in its label can have any quantity. The forwarder holds the tokens of its own resources and of legacy-forwarder resources, because the balances of the legacy forwarder moved to it. The build takes the legacy forwarder from `env/<cluster>.env` (`LEGACY_FORWARDER_PROGRAM_ID`), the zero key on a cluster without one, where it accepts only its own resources.
- **The forwarder lists the logic ref.** `reinitialize` adds the logic refs that the owner passes, and nothing removes them. The owner must list only circuit versions of this forwarder and the logic ref of the legacy forwarder. All applications share the adapter's denylists: a resource with another application's denied logic ref can have this forwarder's label and any quantity, and an emergency migration of it would create resources for tokens that the forwarder does not hold.

## Why the adapter must deny the vulnerable logic ref

An emergency migration does not consume the resource at the adapter; the forwarder records its nullifier. Without the entry in the denylist for consumed resources, the resource owner could also consume the resource at the adapter, for example to soft-migrate it. A transaction that uses the flaw could also add the nullifier of another owner's resource to the adapter's nullifier set, and that resource could then not emergency-migrate. Without the entry in the denylist for created resources, a transaction that uses the flaw could still create resources with the vulnerable logic ref, and these resources could emergency-migrate too. So the logic ref must be on both denylists: deprecating it, which adds it to the denylist for created resources only, is not enough. The adapter has no instruction that removes a denylist entry. If an upgrade of the adapter removes one, every emergency migration fails until the owner denies the logic ref again.

## Limits

- **No amount check.** The tokens stay in escrow, so the balance check of wrap and unwrap cannot check an emergency migration. The trigger's logic must make sure that the created resources match the migrated resources: the same token, the total quantity, and the owner's signature over the action tree root, which covers the commitments of the created resources. If the logic does not check this, an emergency migration can create resources for tokens that the forwarder does not hold.
- **The owner chooses the list.** The forwarder cannot check on chain that it holds the tokens of the resources with a listed logic ref, and no instruction removes a listed logic ref (see [Which resources can emergency-migrate](#which-resources-can-emergency-migrate)).
- **Resources that an attacker created or stole can emergency-migrate.** Nobody can tell them from correct resources: resources are private, and a transaction that uses the flaw looks correct on chain. An earlier root would exclude them, but it needs the start of the attack, which is usually unknown, and it would also block every correct resource created after that root.
- **Soft migration carries a flaw into the active version.** Until the adapter denies a deprecated version with a flaw, resources that an attacker created or stole with the flaw can soft-migrate to the active version.
- **No replacement of withdrawn tokens.** If someone used the flaw to unwrap tokens, the escrow holds fewer tokens than the resources represent. Unwraps then succeed in the order they execute until the escrow does not hold enough tokens, and the unwraps after that fail.
- **Consumed resources cannot emergency-migrate.** A resource whose nullifier the adapter holds cannot emergency-migrate. This includes resources whose nullifier an attacker added with the flaw.
- **A batch fits one settlement.** Each entry adds three accounts to the call's segment, and the call commits the segment's size as a `u8`.

## Notes on the build

- The build is a Cargo feature of the forwarder program rather than a separate program, because an upgrade on Solana replaces a program's code at its address, as the EVM proxy's upgrade replaces its implementation. `dev.sh test-program` writes it for the integration tests (`crates/integration-test/tests/emergency_migrating.rs`); `dev.sh typecheck` checks its IDL (`crates/client/idl/emergency_migrating_spl_token_forwarder.json`).
- Its `CONFIG_VERSION` is one more than the production build's, as the EVM draft's `reinitializer(2)` is one more than `ERC20Forwarder.initialize`'s version. `reinitialize` is owner-only: if the owner does not call it right after the upgrade, nobody else can choose the logic ref and the list.
- Its `reinitialize` replaces the production build's, which takes only a logic ref; the EVM draft's `reinitialize` likewise has the vulnerable list.
- The production build accepts no `Migrate` call (`UnknownOperation`), as the EVM `ERC20Forwarder`'s call type has no `Migrate`.
