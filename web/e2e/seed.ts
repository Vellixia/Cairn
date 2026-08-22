/**
 * Seed a shared project through the real API, exactly as a linked daemon
 * would: a task, a session, a handoff and a memory — with provenance
 * references and no observation content (FR-055).
 */
import { execFile } from "node:child_process";
// Aliased: this file already uses `path` as a parameter name for API paths.
import * as nodePath from "node:path";
import { promisify } from "node:util";

export const API = process.env.NEXT_PUBLIC_CAIRN_API ?? "http://127.0.0.1:8080";

function uuid(): string {
  return crypto.randomUUID();
}

const execFileAsync = promisify(execFile);

/**
 * Path to the `cairn-server` binary, resolved against the repository root
 * rather than `process.cwd()` — Playwright runs from `web/`, so a bare
 * relative path would look for the binary under `web/target/...`. An
 * absolute `CAIRN_SERVER_BIN` (as CI sets) passes through unchanged, since
 * `path.resolve` discards the earlier segments once it hits one.
 */
const SERVER_BIN = nodePath.resolve(
  __dirname,
  "..",
  "..",
  process.env.CAIRN_SERVER_BIN ?? "./target/release/cairn-server",
);

/**
 * Create an account out-of-band via `cairn-server users add`.
 *
 * Self-registration (`POST /api/auth/register`) was removed as a security
 * fix — it was an unauthenticated endpoint whose only validation was a
 * password-length check, and the first step of a full compromise chain
 * (register, discover a project, join it, read and write everything). The
 * replacement is this operator-only subcommand: it talks to the database
 * directly and is not a network route, so tests shell out to the built
 * binary instead of calling an HTTP endpoint.
 */
async function createUser(
  email: string,
  password: string,
  displayName: string,
): Promise<void> {
  const args = [
    "users",
    "add",
    "--email",
    email,
    "--display-name",
    displayName,
    "--password",
    password,
  ];
  if (process.env.DATABASE_URL) {
    args.push("--database-url", process.env.DATABASE_URL);
  }
  try {
    await execFileAsync(SERVER_BIN, args);
  } catch (error) {
    const stderr =
      error && typeof error === "object" && "stderr" in error
        ? String((error as { stderr: unknown }).stderr)
        : String(error);
    throw new Error(`cairn-server users add for ${email} failed: ${stderr}`);
  }
}

/** The server's web session cookie — `auth::COOKIE_NAME` on the Rust side. */
export const SESSION_COOKIE = "cairn_session";

/**
 * Call the API and throw on any non-2xx.
 *
 * Exported so tests never hand-roll a bare `fetch`: an unchecked setup call
 * fails silently and leaves the test asserting against a state it never
 * reached — which, for a test about *refusing* access, is a pass for the
 * wrong reason.
 */
export async function apiJson(path: string, init: RequestInit) {
  const response = await fetch(`${API}${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...(init.headers ?? {}) },
  });
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(`${path}: ${response.status} ${JSON.stringify(body)}`);
  }
  return body;
}

const json = apiJson;

/**
 * Create a user out-of-band (see `createUser` above), sign them in, and
 * return their session cookie value.
 *
 * Throws if either step fails, and if the response carries no session — an
 * empty cookie is indistinguishable from being signed out, so a test that
 * accepted one could not tell "refused because not a member" from "refused
 * because not logged in".
 */
export async function registerAndLogin(
  email: string,
  password: string,
  displayName: string,
): Promise<string> {
  await createUser(email, password, displayName);

  const login = await fetch(`${API}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  if (!login.ok) {
    throw new Error(`login for ${email}: ${login.status}`);
  }

  const pair = (login.headers.get("set-cookie") ?? "").split(";")[0];
  const eq = pair.indexOf("=");
  // Split on the *first* `=` only: the value is opaque and may contain one.
  const value = eq === -1 ? "" : pair.slice(eq + 1);
  if (!value) {
    throw new Error(`login for ${email} returned no ${SESSION_COOKIE} value`);
  }
  return value;
}

/** Mint an API token for a signed-in user. Throws on failure. */
export async function newToken(sessionValue: string, name: string): Promise<string> {
  const body = await apiJson("/api/tokens", {
    method: "POST",
    headers: { cookie: `${SESSION_COOKIE}=${sessionValue}` },
    body: JSON.stringify({ name }),
  });
  const token = body.token as string | undefined;
  if (!token) {
    throw new Error(`/api/tokens returned no token: ${JSON.stringify(body)}`);
  }
  return token;
}

/** Create a shared project with a bearer token. Throws on failure. */
export async function createProject(
  token: string,
  name: string,
  repositoryRemote: string,
): Promise<string> {
  const project = await apiJson("/api/projects", {
    method: "POST",
    headers: { authorization: `Bearer ${token}` },
    body: JSON.stringify({ name, repository_remote: repositoryRemote }),
  });
  const id = project.id as string | undefined;
  if (!id) {
    throw new Error(`/api/projects returned no id: ${JSON.stringify(project)}`);
  }
  return id;
}

export interface Seeded {
  email: string;
  password: string;
  projectId: string;
  sessionId: string;
  memoryContent: string;
}

export async function seed(): Promise<Seeded> {
  const email = `ui-${Date.now()}-${Math.floor(Math.random() * 1e6)}@example.test`;
  const password = "hunter2hunter2";

  await createUser(email, password, "UI Test");

  const login = await fetch(`${API}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  const cookie = (login.headers.get("set-cookie") ?? "").split(";")[0];

  const tokenBody = await json("/api/tokens", {
    method: "POST",
    headers: { cookie },
    body: JSON.stringify({ name: "ui-test" }),
  });
  const token = tokenBody.token as string;
  const auth = { authorization: `Bearer ${token}` };

  const project = await json("/api/projects", {
    method: "POST",
    headers: auth,
    body: JSON.stringify({
      name: `UI Fixture ${Date.now()}`,
      repository_remote: "github.com/example/ui-fixture",
    }),
  });
  const projectId = project.id as string;

  const taskId = uuid();
  const sessionId = uuid();
  const handoffId = uuid();
  const memoryId = uuid();
  const memoryContent = "Errors are returned, never logged and swallowed";

  await json("/api/sync/batch", {
    method: "POST",
    headers: auth,
    body: JSON.stringify({
      project_id: projectId,
      items: [
        {
          idempotency_key: `t-${taskId}`,
          entity_type: "task",
          entity_id: taskId,
          operation: "upsert",
          payload: {
            title: "Add rate limiting",
            goal: "Requests over the limit get 429",
            acceptance_criteria: ["429 returned above threshold"],
            status: "in_progress",
          },
        },
        {
          idempotency_key: `s-${sessionId}`,
          entity_type: "session",
          entity_id: sessionId,
          operation: "upsert",
          payload: {
            task_id: taskId,
            agent: "claude-code",
            branch: "main",
            commit_sha: "abc1234",
            status: "completed",
            started_at: new Date().toISOString(),
            ended_at: new Date().toISOString(),
            end_reason: "clear",
          },
        },
        {
          idempotency_key: `h-${handoffId}`,
          entity_type: "handoff",
          entity_id: handoffId,
          operation: "upsert",
          payload: {
            session_id: sessionId,
            trigger: "session_end",
            goal: "Requests over the limit get 429",
            progress: "1 file changed, 1 test command run, 1 failure open",
            completed_work: ["Changed 1 file(s): src/limiter.rs"],
            remaining_work: ["Open failure: Test failed: cargo test"],
            changed_files: ["src/limiter.rs"],
            decisions: ["Chose a token bucket"],
            failures: ["Test failed: cargo test"],
            tests_executed: [{ command: "cargo test", outcome: "failed" }],
            repository_state: {
              branch: "main",
              commit_sha: "abc1234",
              staged: 0,
              unstaged: 1,
              untracked: 0,
            },
            next_step: "Fix the open failure: Test failed: cargo test",
            evidence: { observation_ids: [uuid()], evidence_count: 1 },
          },
        },
        {
          idempotency_key: `m-${memoryId}`,
          entity_type: "memory",
          entity_id: memoryId,
          operation: "upsert",
          payload: {
            type: "convention",
            scope: "project",
            scope_key: projectId,
            content: memoryContent,
            state: "active",
            provenance: {
              session_id: sessionId,
              observation_ids: [],
              evidence_count: 0,
            },
          },
        },
      ],
    }),
  });

  return { email, password, projectId, sessionId, memoryContent };
}
