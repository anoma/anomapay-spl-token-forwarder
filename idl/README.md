# The adapter's IDL

`protocol_adapter.json` is the production IDL of the Solana protocol adapter at the revision `Cargo.toml` pins (`protocol-adapter`), built with that repository's `./scripts/dev.sh release-build`. The operator scripts read the adapter's seeds from it (`client/constants.ts`): the adapter's state account, whose pause the forwarder's emergency calls check, and the upgrade-authority seed both programs use. Replace it when the pin moves.
