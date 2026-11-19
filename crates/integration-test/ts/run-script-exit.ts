/**
 * An operator script for `operator_scripts.rs`: its main prints "done" and
 * finishes (or throws, when OUTCOME is "reject") while a handle stays open, as
 * web3.js's reconnecting websocket does. `runScript` must still exit.
 */
import { requireEnv, runScript } from "../../../scripts/cli-utils";

setInterval(() => {}, 1000);
runScript("the script failed", async () => {
  console.log("done");
  if (requireEnv("OUTCOME", "resolve or reject") === "reject") throw new Error("boom");
});
