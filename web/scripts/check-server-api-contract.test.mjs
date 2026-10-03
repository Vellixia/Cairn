import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../..", import.meta.url));

test("generated web API declarations are exact v1 server contract", async () => {
  const [contract, generated] = await Promise.all([
    readFile(`${root}/crates/cairn-server/contracts/web-api-v1.ts`, "utf8"),
    readFile(`${root}/web/lib/generated/server-api-v1.ts`, "utf8"),
  ]);
  assert.match(contract, /API_CONTRACT_VERSION = "v1"/);
  assert.equal(generated, `// Generated from crates/cairn-server/contracts/web-api-v1.ts. Do not edit.\n${contract}`);
});
