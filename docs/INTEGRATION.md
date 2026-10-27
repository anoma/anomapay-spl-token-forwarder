# Integrating with the SPL token forwarder

## Events

The forwarder emits its events as the adapter emits its own ([solana-protocol-adapter's `docs/INTEGRATION.md`](https://github.com/anoma/solana-protocol-adapter/blob/anthony/arm-v2-port/solana-pa-prototype/docs/INTEGRATION.md)): as inner instructions whose program is the forwarder, with the same tag, discriminator and Borsh layout.

- During a settlement's wrap or unwrap call: `Wrapped { token, from, amount: u64 }` and `Unwrapped { token, to, amount: u64 }`, the EVM forwarder's events, `token` being the SPL mint.
- From its own instructions: `Initialized { version: u64 }`, `OwnershipTransferred { previous_owner, new_owner }` (`initialize` from the zero key, `transfer_ownership`, `renounce_ownership` to the zero key) and `Upgraded { implementation_hash: [u8;32] }` (`upgrade`, as the adapter's `Upgraded`). `initialize` emits `OwnershipTransferred` before `Initialized`, as the EVM forwarder's initializer does.
- From the emergency-migrating build ([EMERGENCY_MIGRATION.md](EMERGENCY_MIGRATION.md)), as the EVM draft `EmergencyMigratingERC20Forwarder`'s: `VulnerableLogicRefListed { logic_ref }` and `LogicRefReplaced { previous_logic_ref, new_logic_ref }` from its `reinitialize`, and `Migrated { token, vulnerable_logic_ref, nullifier }` during a settlement's migrate call. A migration moves no tokens: an indexer adding up the `Wrapped` and `Unwrapped` amounts must not count it.

Every forwarder instruction that emits one carries the forwarder's event authority (`["__event_authority"]` under the forwarder) and the forwarder's own address; in a settlement they follow the config and the instructions sysvar in the forwarder's call segment, which makes a wrap segment 10 accounts, an unwrap segment 9 and a migrate segment 7 plus 3 per migrated resource. The client crate (`crates/client`) and the TypeScript package (`ts/`) decode the events and build the segments.
