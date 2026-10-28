import { BorshCoder } from "@anchor-lang/core";

import { emergencyMigratingIdl, idl } from "./idl.js";

/** The production build's accounts, events, instructions and types, encoded and decoded as its IDL describes them. */
export const coder = new BorshCoder(idl);

/** The emergency-migrating build's, as its IDL describes them. */
export const emergencyMigratingCoder = new BorshCoder(emergencyMigratingIdl);
