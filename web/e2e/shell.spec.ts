import { expect, test } from "@playwright/test";
import { openNav } from "./nav";
import { seed, type Seeded } from "./seed";

/**
 * The application shell: how someone gets in, moves around, and is stopped
 * from doing damage by accident.
 *
 * Deliberately narrow. These cover the paths the rebuilt interface introduced
 * — sign-in, redirects, navigation, tokens, confirmations — not every screen.
 */
let fixture: Seeded;

test.beforeAll(async () => {
  fixture = await seed();
});

async function signIn(page: import("@playwright/test").Page, f: Seeded) {
  await page.goto("/login");
  await page.getByTestId("email").fill(f.email);
  await page.getByTestId("password").fill(f.password);
  await page.getByTestId("submit").click();
  await expect(page.getByTestId("project-list")).toBeVisible();
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
  await page.goto("/tokens");
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

test("navigation reaches every project section and says where you are", async ({
  page,
}, testInfo) => {
  await signIn(page, fixture);

  await openNav(page, testInfo);
  await page.getByTestId("nav-projects").click();
  await page.getByText("UI Fixture").first().click();

  for (const [testid, heading] of [
    ["nav-tasks", "Tasks"],
    ["nav-sessions", "Sessions"],
    ["nav-memory", "Memory"],
    ["nav-sync", "Sync status"],
  ] as const) {
    await openNav(page, testInfo);
    await page.getByTestId(testid).click();
    await expect(page.getByRole("heading", { name: heading })).toBeVisible();
    // The breadcrumb is the only thing naming the project on a small screen.
    await expect(page.getByRole("navigation").first()).toContainText("Projects");
  }

  await expect(page).toHaveTitle(/Sync · Cairn/);
});

test("the mobile sidebar opens as a sheet", async ({ page }, testInfo) => {
  test.skip(
    testInfo.project.name !== "mobile-chromium",
    "the sheet only exists below the mobile breakpoint",
  );
  await signIn(page, fixture);

  await expect(page.getByTestId("nav-tokens")).toBeHidden();
  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  await expect(page.getByTestId("nav-tokens")).toBeVisible();
  await expect(page.getByTestId("nav-projects")).toBeVisible();
});

test("a token can be created, is shown once, and is revoked behind a confirmation", async ({
  page,
}, testInfo) => {
  await signIn(page, fixture);
  await page.goto("/tokens");
  await expect(page).toHaveTitle(/API tokens · Cairn/);

  const name = `e2e-${testInfo.project.name}-${Date.now()}`;
  await page.getByTestId("new-token").click();
  await page.getByTestId("token-name").fill(name);
  await page.getByTestId("create-token").click();

  // The plaintext exists exactly once, so the panel has to actually show it.
  const revealed = page.getByTestId("revealed-token");
  await expect(revealed).toBeVisible();
  const plaintext = await page.getByTestId("token-plaintext").innerText();
  expect(plaintext).toHaveLength(64);

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
