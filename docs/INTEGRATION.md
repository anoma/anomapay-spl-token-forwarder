# Integrating with the SPL token forwarder

## Events

The forwarder emits its events as the adapter emits its own ([solana-protocol-adapter's `docs/INTEGRATION.md`](https://github.com/anoma/solana-protocol-adapter/blob/anthony/arm-v2-port/solana-pa-prototype/docs/INTEGRATION.md)): as inner instructions whose program is the forwarder, with the same tag, discriminator and Borsh layout.

- During a settlement's wrap or unwrap call: `Wrapped { token, from, amount: u64 }` and `Unwrapped { token, to, amount: u64 }`, the EVM forwarder's events, `token` being the SPL mint.
- From its own instructions: `Initialized { version: u64 }`, `OwnershipTransferred { previous_owner, new_owner }` (`initialize` from the zero key, `transfer_ownership`, `renounce_ownership` to the zero key), `Upgraded { executable_hash: [u8;32] }` (`upgrade`, as the adapter's `UpgradedEvent`), `EmergencyCallerSet { emergency_caller, set_by }` and `EmergencyWithdraw { token_mint, to, amount: u64, caller }`. `initialize` emits `OwnershipTransferred` before `Initialized`, as the EVM forwarder's initializer does.

Every forwarder instruction that emits one carries the forwarder's event authority (`["__event_authority"]` under the forwarder) and the forwarder's own address; in a settlement they follow the config and the instructions sysvar in the forwarder's call segment, which makes a wrap segment 10 accounts and an unwrap segment 9. The client crate (`crates/client`) and the TypeScript package (`ts/`) decode the events and build both segments.
