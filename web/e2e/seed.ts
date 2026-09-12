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
            // `runner`, not `command`. The server's wire check screens field
            // *names* recursively, so a `command` key anywhere inside a handoff
            // payload is refused outright (FR-532) — this seed's handoff never
            // landed, and the session page then had no handoff to render at all.
            tests_executed: [{ runner: "cargo test", outcome: "failed" }],
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

// ---------------------------------------------------------------------------
// Feature 005, User Story 5 — the control-plane fixture (T107)
// ---------------------------------------------------------------------------
//
// **Everything below is seeded over HTTP, and nothing below touches the
// database or a log file.** That restriction is the whole point of SC-727: if
// the path from a session to a retrieval can only be followed with `psql`, the
// control plane has not been built. So this fixture opens a session, posts safe
// events, and then *waits for the server's own consolidation task* to turn them
// into a run, a candidate and a memory — rather than inserting those rows the
// way `tests/tests/feature005_control_plane_api.rs` legitimately does, since
// that suite is measuring the read API and this one is measuring the path.
//
// What that costs: the fixture depends on a deployment whose consolidation task
// actually runs — schema ≥ 4 and a pool of at least five connections
// (`consolidate::pool_share`), which the default `--max-connections 10` gives.
// What it buys: every fact the browser tests read was produced by the pipeline
// under test, so a chain that renders is a chain that exists.

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";

/** `cairn_core::eventid::CAIRN_EVENT_NS`. */
const CAIRN_EVENT_NS = "1e7a0c51-4b9d-5a2e-9f38-6d41c80b2711";

/** `cairn_core::eventid::SEP` — the byte no name component can contain. */
const NAME_SEPARATOR = 0x1f;

function uuidToBytes(id: string): Buffer {
  return Buffer.from(id.replace(/-/g, ""), "hex");
}

function bytesToUuid(bytes: Buffer): string {
  const hex = bytes.toString("hex");
  return [
    hex.slice(0, 8),
    hex.slice(8, 12),
    hex.slice(12, 16),
    hex.slice(16, 20),
    hex.slice(20, 32),
  ].join("-");
}

/**
 * UUIDv5, the same five lines RFC 4122 specifies.
 *
 * Hand-rolled rather than pulled from a package because the suite has no uuid
 * dependency and this is a SHA-1 digest with two bits set — a dependency for it
 * would be more surface than the function.
 */
function uuidV5(namespace: string, name: Buffer): string {
  const digest = createHash("sha1")
    .update(uuidToBytes(namespace))
    .update(name)
    .digest();
  const out = digest.subarray(0, 16);
  out[6] = (out[6] & 0x0f) | 0x50;
  out[8] = (out[8] & 0x3f) | 0x80;
  return bytesToUuid(out);
}

/**
 * `event_id = UUIDv5(CAIRN_EVENT_NS, session_id ‖ 0x1f ‖ session_seq)`.
 *
 * The server re-derives this and refuses a mismatch (`events.rs` §7.1 step 4),
 * so a client cannot choose its own event identity — which also means a bug in
 * this function shows up as `identity_mismatch` on ingest rather than as a
 * mysteriously empty feed. `assertAllAccepted` below is what turns that into a
 * loud failure.
 */
function eventId(sessionId: string, seq: number): string {
  const name = Buffer.concat([
    uuidToBytes(sessionId),
    Buffer.from([NAME_SEPARATOR]),
    Buffer.from(String(seq), "ascii"),
  ]);
  return uuidV5(CAIRN_EVENT_NS, name);
}

/** One signed-in account, with both credentials the fixture needs. */
export interface Account {
  email: string;
  password: string;
  displayName: string;
  /** The `cairn_session` cookie value, for driving the browser. */
  session: string;
  /** A bearer token, for seeding and for calling the API directly. */
  token: string;
  id: string;
}

/** Call the API with a bearer token and throw on any non-2xx. */
export async function apiAs(token: string, path: string, init: RequestInit = {}) {
  return apiJson(path, {
    ...init,
    headers: { authorization: `Bearer ${token}`, ...(init.headers ?? {}) },
  });
}

/**
 * Call the API and return the status **without** throwing.
 *
 * Exported for the refusal assertions: a test that proves a member cannot
 * ratify needs the status code, and `apiAs` would turn the very thing it is
 * asserting into a thrown error.
 */
export async function apiStatusAs(
  token: string,
  path: string,
  init: RequestInit = {},
): Promise<number> {
  const response = await fetch(`${API}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${token}`,
      ...(init.headers ?? {}),
    },
  });
  return response.status;
}

/** Create an account, sign it in, mint it a token, and learn its id. */
async function account(label: string): Promise<Account> {
  const email = `${label}-${Date.now()}-${Math.floor(Math.random() * 1e6)}@example.test`;
  const password = "hunter2hunter2";
  const displayName = `US5 ${label}`;
  const session = await registerAndLogin(email, password, displayName);
  const token = await newToken(session, `${label}-us5`);
  const me = await apiAs(token, "/api/auth/me");
  return { email, password, displayName, session, token, id: me.id as string };
}

/**
 * Bring an administrator into existence, out of band.
 *
 * **There is no route that can do this, and that is deliberate.** Account
 * creation is operator-only (`cairn-server users add`), and that subcommand
 * creates a *member*: `role` is only ever set to `admin` by
 * `auth::ensure_admin`, which runs at start-up from `CAIRN_ADMIN_EMAIL` /
 * `CAIRN_ADMIN_PASSWORD`. On a database with no accounts — which is what a
 * fresh CI deployment is — migration 3's backfill has nobody to promote either,
 * so a stack started without those variables has **no administrator at all**.
 *
 * So the fixture performs the same deploy-time act the contract reserves for an
 * operator (`web-control-plane.md` §1.1): it runs the server binary once, on an
 * ephemeral port, with the environment account configured. `ensure_admin` is an
 * upsert, so this converges whether or not the account already exists, and the
 * process is killed the moment the credential authenticates against the *real*
 * server — which is also the proof that the account landed in the shared
 * database rather than somewhere the tests cannot see.
 *
 * If the stack was already started with those variables set, this is a no-op
 * that re-establishes the same password.
 */
async function bootstrapAdmin(): Promise<Account> {
  const email = (
    process.env.CAIRN_ADMIN_EMAIL ?? `us5-admin-${Date.now()}@example.test`
  )
    .trim()
    .toLowerCase();
  const password = process.env.CAIRN_ADMIN_PASSWORD ?? "hunter2hunter2";
  const displayName = "US5 Administrator";

  const args = ["--addr", "127.0.0.1:0"];
  if (process.env.DATABASE_URL) {
    args.push("--database-url", process.env.DATABASE_URL);
  }
  const child = spawn(SERVER_BIN, args, {
    env: {
      ...process.env,
      CAIRN_ADMIN_EMAIL: email,
      CAIRN_ADMIN_PASSWORD: password,
      CAIRN_ADMIN_DISPLAY_NAME: displayName,
    },
    stdio: "ignore",
  });

  let stderr = "";
  child.on("error", (error) => {
    stderr = String(error);
  });

  try {
    const deadline = Date.now() + 60_000;
    for (;;) {
      const login = await fetch(`${API}/api/auth/login`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email, password }),
      }).catch(() => null);
      if (login?.ok) {
        const pair = (login.headers.get("set-cookie") ?? "").split(";")[0];
        const eq = pair.indexOf("=");
        const session = eq === -1 ? "" : pair.slice(eq + 1);
        if (!session) {
          throw new Error(`admin login returned no ${SESSION_COOKIE} value`);
        }
        const token = await newToken(session, "us5-admin");
        const me = await apiAs(token, "/api/auth/me");
        if (me.role !== "admin") {
          throw new Error(
            `the environment account came back as ${me.role}, not admin: ${JSON.stringify(me)}`,
          );
        }
        return {
          email,
          password,
          displayName,
          session,
          token,
          id: me.id as string,
        };
      }
      if (Date.now() > deadline) {
        throw new Error(
          `no administrator after 60s. The bootstrap server did not seed ` +
            `${email}${stderr ? `: ${stderr}` : ""}. ` +
            `Start the stack with CAIRN_ADMIN_EMAIL and CAIRN_ADMIN_PASSWORD, ` +
            `or check that ${SERVER_BIN} exists.`,
        );
      }
      await new Promise((r) => setTimeout(r, 250));
    }
  } finally {
    child.kill();
  }
}

/** One safe event, in the shape `POST /api/events/batch` accepts. */
function event(
  session: string,
  seq: number,
  kind: string,
  content: unknown,
): Record<string, unknown> {
  return {
    event_id: eventId(session, seq),
    contract_version: 1,
    kind,
    agent: "claude_code",
    vendor_event: "PostToolUse",
    session_id: session,
    session_seq: seq,
    occurred_at: new Date().toISOString(),
    content,
  };
}

/**
 * Post a batch and insist every event landed.
 *
 * A per-event refusal comes back inside a `200`, so an unchecked ingest leaves
 * the whole fixture asserting against events that were never stored — and the
 * failure surfaces two minutes later as "consolidation never produced
 * anything", which points at the wrong thing entirely.
 */
async function ingest(token: string, events: Record<string, unknown>[]) {
  const body = await apiAs(token, "/api/events/batch", {
    method: "POST",
    body: JSON.stringify({ contract_version: 1, events }),
  });
  const results = (body.results ?? []) as { status: string; reason?: string }[];
  const bad = results.filter((r) => r.status !== "accepted");
  if (bad.length > 0 || results.length !== events.length) {
    throw new Error(
      `not every safe event was accepted: ${JSON.stringify(body)}`,
    );
  }
}

/** Everything the US5 browser tests read, and how each part came to exist. */
export interface ControlPlaneFixture {
  /** Ordinary member; owns the project, the events and the retrieval. */
  owner: Account;
  /** Ordinary member of the *same* project — the domain-privacy counterparty. */
  mate: Account;
  /** Server administrator, and deliberately not a member of the project. */
  admin: Account;
  projectId: string;
  projectName: string;
  /** The session whose events consolidation turned into knowledge. */
  sessionId: string;
  /** The memory the accepted candidate produced. */
  knowledgeId: string;
  /** Its content — the exact sentence R7 writes. */
  knowledgeContent: string;
  /** `knowledge:project:<id>`, the complete two-part reference. */
  referenceKey: string;
  /**
   * A command line that reached the server as a safe event.
   *
   * The memory detail page must never render it: the evidence behind a record
   * is local to the machine that captured it (FR-893).
   */
  evidenceCommand: string;
  /** The owner's retrieval over that knowledge. */
  traceId: string;
  /** The owner's personal note — invisible to `mate`. */
  personalNote: string;
  /** The owner's pattern — invisible to `mate`. */
  patternTitle: string;
  /** A team proposal, for the ratify/retire transitions. */
  teamId: string;
  teamContent: string;
}

/**
 * Seed the whole `session → event → run → candidate → knowledge → retrieval`
 * path over HTTP, and wait for the server to walk the middle of it.
 */
export async function seedControlPlane(): Promise<ControlPlaneFixture> {
  const admin = await bootstrapAdmin();
  const owner = await account("owner");
  const mate = await account("mate");
  // One token that makes every fixture string unique to this run.
  const run = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

  const projectName = `US5 Control Plane ${run}`;
  const projectId = await createProject(
    owner.token,
    projectName,
    `github.com/example/us5-${Date.now()}`,
  );
  await apiAs(owner.token, `/api/projects/${projectId}/members`, {
    method: "POST",
    body: JSON.stringify({ user_id: mate.id }),
  });

  // A closed session, so consolidation elects it at once rather than waiting
  // out the ten-minute age threshold a still-open session is held to.
  const sessionId = uuid();
  await apiAs(owner.token, "/api/sync/batch", {
    method: "POST",
    body: JSON.stringify({
      project_id: projectId,
      items: [
        {
          idempotency_key: `s-${sessionId}`,
          entity_type: "session",
          entity_id: sessionId,
          operation: "upsert",
          payload: {
            agent: "claude-code",
            branch: "main",
            commit_sha: "abc1234",
            status: "completed",
            started_at: new Date().toISOString(),
            ended_at: new Date().toISOString(),
            end_reason: "clear",
          },
        },
      ],
    }),
  });

  // The four events, in the order the vocabulary rule requires.
  //
  // A `decision_signal`'s tokens must be *justified* by events the server
  // already holds with a lower `session_seq` (`extraction.md` §13.3) — a token
  // nothing established is refused permanently. So the file change comes first
  // and supplies `ledger` (a directory: a module token) and `marmoset` (the
  // file), which the decision at seq 3 then cites. Get this order wrong and
  // ingest answers `token_not_in_vocabulary`, which `ingest` above turns into a
  // failure here rather than into an empty activity feed later.
  const evidenceCommand = "cargo bench --bench crimson_pillar_probe";
  await ingest(owner.token, [
    event(sessionId, 1, "file_changed", {
      File: {
        repo_file: "ledger/marmoset.rs",
        repo_file_from: null,
        change_kind: "modified",
        file_identity: "present",
      },
    }),
    event(sessionId, 2, "command_executed", {
      Command: { command_line: evidenceCommand, exit_status: 0 },
    }),
    event(sessionId, 3, "decision_signal", {
      Decision: {
        decision_kind: "adopt",
        subject_token: "ledger",
        object_token: "marmoset",
        justified_by_seq: 1,
        lexicon_version: 1,
      },
    }),
    event(sessionId, 4, "session_closed", {
      SessionClose: { close_reason: "clear" },
    }),
  ]);

  const { knowledgeId, knowledgeContent } = await awaitConsolidation(
    owner.token,
    projectId,
  );

  // The two owner-only domains, and a team proposal for the admin to act on.
  // None of the three may name a project (FR-822), so the content is written
  // to be about nothing in particular.
  const personalNote =
    "I prefer to read the failing assertion before the stack trace.";
  await apiAs(owner.token, "/api/personal/knowledge", {
    method: "POST",
    body: JSON.stringify({
      type: "convention",
      content: personalNote,
      topic_key: "review.order",
      value_key: "assertion_first",
    }),
  });

  const patternTitle = "Quiet flake, loud fixture";
  await apiAs(owner.token, "/api/patterns", {
    method: "POST",
    body: JSON.stringify({
      title: patternTitle,
      problem: "A test fails only when the suite is run in parallel.",
      root_cause: "Two workers share one temporary directory.",
      approach: "Give each worker its own directory and assert on the path.",
    }),
  });

  // Unique per fixture, and it has to be. Both Playwright projects run this
  // spec against one server, so two fixtures exist at once — a fixed string
  // gives them the same team content, a row filter matches both, and the
  // assertion fails on the count rather than on anything about ratification.
  const teamContent = `Every irreversible action asks once before it happens. [${run}]`;
  const proposal = await apiAs(owner.token, "/api/team/knowledge", {
    method: "POST",
    body: JSON.stringify({
      type: "convention",
      content: teamContent,
      topic_key: "confirmation.irreversible",
      value_key: "ask_once",
    }),
  });
  const teamId = proposal.id as string;

  // The retrieval runs last, so the knowledge and the pattern both exist to be
  // selected — which is what makes the trace carry a `knowledge` reference
  // *and* a `pattern` reference, the two reference shapes the UI must render
  // differently.
  const retrieval = await apiAs(owner.token, "/api/retrieve", {
    method: "POST",
    body: JSON.stringify({ session_id: sessionId, trigger: "session_open" }),
  });
  const traceId = retrieval.trace_id as string;
  if (!traceId) {
    throw new Error(`/api/retrieve returned no trace: ${JSON.stringify(retrieval)}`);
  }

  return {
    owner,
    mate,
    admin,
    projectId,
    projectName,
    sessionId,
    knowledgeId,
    knowledgeContent,
    referenceKey: `knowledge:project:${knowledgeId}`,
    evidenceCommand,
    traceId,
    personalNote,
    patternTitle,
    teamId,
    teamContent,
  };
}

/**
 * Wait for the server's consolidation task to accept a candidate, and read back
 * what it produced — over the activity feed, like any other client.
 *
 * **Polling a read API rather than the database is the point.** SC-727 is the
 * claim that the path is reconstructible without one, and a fixture that
 * watched `knowledge_candidates` directly would have quietly exempted itself
 * from the property the tests exist to prove.
 *
 * The deadline is generous because consolidation shares a process with request
 * serving and elects one session at a time across the whole deployment: on a
 * busy database this session waits behind others. A timeout here is a real
 * finding — either the deployment's pool is too small for consolidation to run
 * at all (`--max-connections` below five), or the pipeline is stuck — so it
 * says which rather than failing as a bare assertion.
 */
async function awaitConsolidation(
  token: string,
  projectId: string,
): Promise<{ knowledgeId: string; knowledgeContent: string }> {
  const deadline = Date.now() + 120_000;
  let lastSeen = "nothing at all";
  for (;;) {
    const feed = await apiAs(
      token,
      `/api/projects/${projectId}/activity?kinds=accepted&limit=100`,
    );
    const items = (feed.items ?? []) as {
      family: string;
      kind: string;
      reference: { knowledge_id: string; domain: string | null } | null;
    }[];
    const accepted = items.find(
      (i) =>
        i.family === "candidate_decision" &&
        i.kind === "accepted" &&
        i.reference?.domain === "project",
    );
    if (accepted?.reference) {
      const knowledgeId = accepted.reference.knowledge_id;
      const detail = await apiAs(token, `/api/memories/${knowledgeId}`);
      return {
        knowledgeId,
        knowledgeContent: detail.memory.content as string,
      };
    }
    lastSeen = JSON.stringify(items);
    if (Date.now() > deadline) {
      const health = await apiAs(token, "/api/consolidation/health").catch(
        (e) => ({ unreachable: String(e) }),
      );
      throw new Error(
        `consolidation accepted nothing within 120s.\n` +
          `activity(accepted) = ${lastSeen}\n` +
          `consolidation health = ${JSON.stringify(health)}\n` +
          `A deployment whose pool is below five connections runs no ` +
          `consolidation task at all (consolidate::pool_share); start ` +
          `cairn-server with at least --max-connections 5.`,
      );
    }
    await new Promise((r) => setTimeout(r, 500));
  }
}
