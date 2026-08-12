import { expect, test } from "@playwright/test";
import { openNav } from "./nav";
import { seed, type Seeded } from "./seed";

/**
 * T073 — the four Independent Test actions for US7, without a terminal:
 * find the project, read its handoff, search memory, delete a memory (SC-011).
 */
let fixture: Seeded;

test.beforeAll(async () => {
  fixture = await seed();
});

test.beforeEach(async ({ page }) => {
  await page.goto("/login");
  await page.getByTestId("email").fill(fixture.email);
  await page.getByTestId("password").fill(fixture.password);
  await page.getByTestId("submit").click();
  await expect(page.getByTestId("project-list")).toBeVisible();
});

test("a teammate finds the project and reads its handoff", async ({ page }, testInfo) => {
  await page.getByText("UI Fixture").first().click();

  await expect(page.getByText("Recent sessions")).toBeVisible();
  await openNav(page, testInfo);
  await page.getByTestId("nav-sessions").click();
  await expect(page.getByTestId("session-list")).toBeVisible();

  await page.getByTestId("session-list").getByRole("listitem").first().click();
  await expect(page.getByTestId("handoff")).toBeVisible();
  await expect(page.getByTestId("next-step")).toContainText("Fix the open failure");
  // The path appears in both the completed summary and the changed-files list.
  await expect(page.getByText("src/limiter.rs").first()).toBeVisible();
  await expect(page.getByText("cargo test").first()).toBeVisible();
  await expect(page.getByTestId("handoff")).toContainText("Tests executed");
});

test("memory is searchable by scope and shows its provenance", async ({ page }) => {
  await page.goto(`/projects/${fixture.projectId}/memory`);
  await expect(page.getByTestId("memory-list")).toBeVisible();

  await page.getByTestId("memory-search").fill("swallowed");
  await expect(page.getByTestId("memory-content").first()).toContainText(
    "never logged and swallowed",
  );

  // The scope filter is a listbox, not a native <select>: open it, then pick.
  await page.getByTestId("scope-filter").click();
  await page.getByRole("option", { name: "project", exact: true }).click();
  await expect(page.getByTestId("memory-content").first()).toBeVisible();

  // Provenance is a session and a count; evidence content lives locally.
  await expect(page.getByTestId("provenance-session").first()).toBeVisible();
  await expect(page.getByTestId("evidence-count").first()).toContainText("evidence");
});

test("a memory can be deleted from the browser", async ({ page }) => {
  await page.goto(`/projects/${fixture.projectId}/memory`);
  const rows = page.getByTestId("memory-list").getByRole("listitem");
  await expect(rows).toHaveCount(1);

  // Deleting is irreversible, so it asks first — and cancelling must keep it.
  // Both paths are asserted here rather than in two tests, because the fixture
  // holds one memory and a separate test would depend on running first.
  await page.getByRole("button", { name: "Delete memory" }).first().click();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(rows).toHaveCount(1);

  await page.getByRole("button", { name: "Delete memory" }).first().click();
  const confirm = page.getByRole("alertdialog");
  await expect(confirm).toContainText("Delete this memory?");
  await confirm.getByTestId("confirm").click();

  await expect(rows).toHaveCount(0);
  await expect(page.getByText("No memory yet")).toBeVisible();
});

test("tasks and sync status are reachable without a terminal", async ({
  page,
}, testInfo) => {
  await page.goto(`/projects/${fixture.projectId}`);
  await openNav(page, testInfo);
  await page.getByTestId("nav-tasks").click();
  await expect(page.getByTestId("task-list")).toContainText("Add rate limiting");
  await page.getByTestId("filter-in_progress").click();
  await expect(page.getByTestId("task-list")).toContainText("Add rate limiting");

  await openNav(page, testInfo);
  await page.getByTestId("nav-sync").click();
  await expect(page.getByTestId("sync-status")).toContainText("items applied");
});

test("a non-member is refused rather than shown an empty page", async ({
  page,
}) => {
  // Register a second user who is NOT a member of the fixture project.
  const stranger = `stranger-${Date.now()}@example.test`;
  const password = "hunter2hunter2";
  await fetch(`${API}/api/auth/register`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: stranger, display_name: "Stranger", password }),
  });
  const login = await fetch(`${API}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: stranger, password }),
  });
  const cookie = (login.headers.get("set-cookie") ?? "").split(";")[0];

  // Set the stranger's cookie and try to access the fixture project.
  await page.context().addCookies([
    { name: "session", value: cookie.split("=")[1] ?? "", domain: "127.0.0.1", path: "/" },
  ]);
  await page.goto(`/projects/${fixture.projectId}`);
  // The UI should show a refusal, not an empty project page.
  await expect(page.getByText(/forbidden|not found|access denied/i).first()).toBeVisible();
});

test("the projects list shows an empty state when there are none", async ({
  page,
}) => {
  // Register a fresh user with no projects.
  const empty = `empty-${Date.now()}@example.test`;
  const password = "hunter2hunter2";
  await fetch(`${API}/api/auth/register`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: empty, display_name: "Empty", password }),
  });
  const login = await fetch(`${API}/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: empty, password }),
  });
  const cookie = (login.headers.get("set-cookie") ?? "").split(";")[0];

  await page.context().addCookies([
    { name: "session", value: cookie.split("=")[1] ?? "", domain: "127.0.0.1", path: "/" },
  ]);
  await page.goto("/");
  await expect(page.getByTestId("project-list")).toBeVisible();
  await expect(page.getByText(/no projects|create a project/i).first()).toBeVisible();
});

test("the sessions list shows an empty state when there are none", async ({
  page,
}, testInfo) => {
  // Create a project with no sessions.
  const tokenBody = await fetch(`${API}/api/tokens`, {
    method: "POST",
    headers: { cookie: `session=${fixture.email}` },
    body: JSON.stringify({ name: "empty-sessions-test" }),
  }).then(r => r.json());
  const token = tokenBody.token as string;
  const auth = { authorization: `Bearer ${token}` };

  const project = await fetch(`${API}/api/projects`, {
    method: "POST",
    headers: { ...auth, "content-type": "application/json" },
    body: JSON.stringify({ name: `Empty Sessions ${Date.now()}`, repository_remote: "github.com/example/empty" }),
  }).then(r => r.json());

  await page.goto(`/projects/${project.id}/sessions`);
  await expect(page.getByTestId("session-list")).toBeVisible();
  await expect(page.getByText(/no sessions|no handoffs/i).first()).toBeVisible();
});
