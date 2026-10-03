import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
const source = `${root}/crates/cairn-server/contracts/web-api-v1.ts`;
const output = `${root}/web/lib/generated/server-api-v1.ts`;
const generated = `// Generated from crates/cairn-server/contracts/web-api-v1.ts. Do not edit.\n${await readFile(source, "utf8")}`;

if (process.argv.includes("--check")) {
  const current = await readFile(output, "utf8").catch(() => "");
  if (current !== generated) {
    console.error("web API contract drift: run npm --prefix web run api-contract:generate");
    process.exitCode = 1;
  }
} else {
  await mkdir(`${root}/web/lib/generated`, { recursive: true });
  await writeFile(output, generated);
}
