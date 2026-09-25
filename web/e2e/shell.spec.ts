import { expect, test } from "@playwright/test";
import { openNav } from "./nav";
import { seed, seedControlPlane, type Seeded } from "./seed";

/**
 * The application shell: how someone gets in, moves around, and is stopped
 * from doing damage by accident.
 *
 * Deliberately narrow. These cover the paths the rebuilt interface introduced
 * — sign-in, redirects, navigation, tokens, confirmations — not every screen.
 */
let fixture: Seeded;
let admin: Awaited<ReturnType<typeof seedControlPlane>>["admin"];

test.beforeAll(async () => {
  fixture = await seed();
  admin = (await seedControlPlane()).admin;
});

async function signIn(
  page: import("@playwright/test").Page,
  f: Pick<Seeded, "email" | "password">,
) {
  await page.goto("/login");
  // Sidebar role-gating is driven by `/api/auth/me`. Register this wait before
  // submitting, so an Accounts-absent assertion cannot win a race against the
  // member response that decides whether the admin link is rendered.
  const authenticated = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === "/api/auth/me" &&
      response.status() === 200,
  );
  await page.getByTestId("email").fill(f.email);
  await page.getByTestId("password").fill(f.password);
  await page.getByTestId("submit").click();
  await authenticated;
  await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible();
}

test("the sign-in page names the deployment and its version", async ({
  page,
}) => {
  await page.goto("/login");
  await expect(page).toHaveTitle(/Sign in · Cairn/);
  await expect(page.getByRole("heading", { name: "Sign in to Cairn" })).toBeVisible();
  // Served without signing in, so a deployment can be identified from outside.
  await expect(page.getByText(/^Cairn v/)).toBeVisible();
});

test("a signed-out visitor asking for a page is sent to sign in", async ({
  page,
}) => {
  await page.goto("/settings");
  await expect(page).toHaveURL(/\/login$/);
  await expect(page.getByTestId("password")).toBeVisible();
});

test("wrong credentials are refused in place, not silently", async ({ page }) => {
  await page.goto("/login");
  await page.getByTestId("email").fill(fixture.email);
  await page.getByTestId("password").fill("definitely-not-the-password");
  await page.getByTestId("submit").click();
  // Named rather than found by role: Next.js keeps its own `role="alert"`
  // route announcer in the page, so asking for the role alone is ambiguous
  // the moment a navigation has happened — and it fails as a strict-mode
  // violation, which reads like a broken page rather than a broken locator.
  await expect(page.getByTestId("login-error")).toBeVisible();
  await expect(page).toHaveURL(/\/login$/);
});

test("a signed-in visitor is not shown the sign-in form again", async ({
  page,
}) => {
  await signIn(page, fixture);
  await page.goto("/login");
  // The form is a dead end once signed in; the bookmark should still work.
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByTestId("project-list")).toBeVisible();
});

test("an unknown address gets Cairn's own 404, not a framework default", async ({
  page,
}) => {
  await page.goto("/no-such-page-exists");
  await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Back to projects" }),
  ).toBeVisible();
});

test("removed workflows have no redirect", async ({ page }) => {
  await page.goto("/projects/not-a-project/tasks");
  await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
  await page.goto("/tokens");
  await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
});

test("compact navigation reaches project workflows and global governance", async ({
  page,
}, testInfo) => {
  await signIn(page, fixture);

  await openNav(page, testInfo);
  await page.getByTestId("nav-projects").click();
  await page.getByTestId("project-list").getByText("UI Fixture").first().click();

  for (const [testid, heading] of [
    ["nav-overview", "UI Fixture"],
    ["nav-memory", "Memory"],
    ["nav-sessions-replay", "Sessions & Replay"],
  ] as const) {
    await openNav(page, testInfo);
    await page.getByTestId(testid).click();
    await expect(page.getByRole("heading", { name: heading })).toBeVisible();
    // The breadcrumb is the only thing naming the project on a small screen.
    await expect(page.getByRole("navigation").first()).toContainText("Projects");
  }

  await openNav(page, testInfo);
  await page.getByTestId("nav-governance").click();
  await expect(page.getByRole("heading", { name: "Governance" })).toBeVisible();
  await openNav(page, testInfo);
  await expect(page.getByTestId("nav-settings")).toBeVisible();
});

test("administrator sees Settings in global navigation", async ({ page }, testInfo) => {
  await signIn(page, admin);
  await openNav(page, testInfo);
  await expect(page.getByTestId("nav-settings")).toBeVisible();
});

test("governance keeps member reads and administrator transitions separate", async ({ page }) => {
  await signIn(page, admin);
  await page.goto("/governance");
  await expect(page.getByTestId("team-list")).toBeVisible();
  await expect(page.getByTestId("team-more")).toHaveCount(0);
});

test("the mobile sidebar opens as a sheet", async ({ page }, testInfo) => {
  test.skip(
    testInfo.project.name !== "mobile-chromium",
    "the sheet only exists below the mobile breakpoint",
  );
  await signIn(page, fixture);

  await expect(page.getByTestId("nav-settings")).toBeHidden();
  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  await expect(page.getByTestId("nav-settings")).toBeVisible();
  await expect(page.getByTestId("nav-projects")).toBeVisible();
});

test("a token can be created, is shown once, and is revoked behind a confirmation", async ({
  page,
}, testInfo) => {
  await signIn(page, fixture);
  await page.goto("/settings");
  await expect(page).toHaveTitle(/Settings · Cairn/);

  const name = `e2e-${testInfo.project.name}-${Date.now()}`;
  await page.getByTestId("new-token").click();
  await page.getByTestId("token-name").fill(name);
  await page.getByTestId("create-token").click();

  // The plaintext exists exactly once, so the panel has to actually show it.
  const revealed = page.getByTestId("revealed-token");
  await expect(revealed).toBeVisible();
  const credential = JSON.parse(
    await page.getByTestId("token-plaintext").innerText(),
  ) as { server_token: string; server_url: string; web_url: string };
  expect(credential.server_token).toHaveLength(64);
  expect(credential.server_url).toBe("http://127.0.0.1:8080");
  expect(credential.web_url).toBe("http://127.0.0.1:3100");

  const row = page.getByTestId("token-row").filter({ hasText: name });
  await expect(row).toContainText("Active");

  // Revoking is irreversible, so it asks.
  await row.getByRole("button", { name: `Revoke ${name}` }).click();
  const confirm = page.getByRole("alertdialog");
  await expect(confirm).toContainText(`Revoke ${name}?`);
  await confirm.getByTestId("confirm").click();
  await expect(row).toContainText("Revoked");
});

test("memory search filters, and clears back to everything", async ({
  page,
}) => {
  await signIn(page, fixture);
  await page.goto(`/projects/${fixture.projectId}/memory`);
  await expect(page.getByTestId("memory-list")).toBeVisible();

  // Typing is debounced: the result follows the pause, not each keystroke.
  await page.getByTestId("memory-search").fill("nothing-matches-this-string");
  await expect(page.getByText("No memory matches")).toBeVisible();

  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(page.getByTestId("memory-content").first()).toBeVisible();
});

test("memory detail keeps evidence, graph, and canonical mutations together", async ({ page }) => {
  await signIn(page, fixture);
  await page.goto(`/projects/${fixture.projectId}/memory`);
  await page.getByTestId("memory-content").first().click();
  await expect(page.getByTestId("memory-detail")).toBeVisible();
  await expect(page.getByTestId("evidence-local-notice")).toBeVisible();
  await expect(page.getByTestId("memory-graph")).toBeVisible();
  await expect(page.getByRole("button", { name: "Reinforce" })).toBeVisible();
  await expect(page.getByLabel("Replacement memory")).toBeVisible();
  await expect(page.getByLabel("Related memory ID")).toBeVisible();
});

test("sessions retain handoff and accepted-event replay", async ({ page }) => {
  await signIn(page, fixture);
  await page.goto(`/projects/${fixture.projectId}/sessions`);
  await expect(page.getByTestId("accepted-event-replay")).toBeVisible();
  await page.getByTestId("session-list").getByRole("listitem").first().click();
  await expect(page.getByTestId("handoff")).toBeVisible();
});

test("settings exposes password and member controls without user enumeration", async ({ page }) => {
  await signIn(page, fixture);
  await page.goto("/settings");
  await expect(page.getByLabel("New password")).toBeVisible();
  await page.getByTestId("settings-projects").getByText("UI Fixture").first().click();
  await expect(page.getByTestId("project-members")).toBeVisible();
  await expect(page.getByLabel("Member user ID")).toBeVisible();
  await expect(page.getByTestId("account-table")).toHaveCount(0);
});

test("administrator can create a setup-ready project from Settings", async ({ page }) => {
  await signIn(page, admin);
  await page.goto("/settings");

  const name = `browser-project-${Date.now()}`;
  const remote = `github.com/example/${name}`;
  const created = page.waitForResponse((response) =>
    new URL(response.url()).pathname === "/api/projects" &&
    response.request().method() === "POST" &&
    response.status() === 200,
  );
  await page.getByLabel("Project name").fill(name);
  await page.getByLabel("Repository remote").fill(remote);
  await page.getByRole("button", { name: "Create project" }).click();
  const project = (await (await created).json()) as { id: string };

  await page.goto(`/projects/${project.id}`);
  await expect(page.getByRole("heading", { name })).toBeVisible();
  await expect(page.getByText(remote)).toBeVisible();
});
