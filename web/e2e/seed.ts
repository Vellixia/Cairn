/**
 * Seed a shared project through the real API, exactly as a linked daemon
 * would: a task, a session, a handoff and a memory — with provenance
 * references and no observation content (FR-055).
 */
export const API = process.env.NEXT_PUBLIC_CAIRN_API ?? "http://127.0.0.1:8080";

function uuid(): string {
  return crypto.randomUUID();
}

async function json(path: string, init: RequestInit) {
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

  await json("/api/auth/register", {
    method: "POST",
    body: JSON.stringify({ email, display_name: "UI Test", password }),
  });

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
