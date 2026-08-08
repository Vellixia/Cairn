import { expect, test } from "@playwright/test";
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

test("a teammate finds the project and reads its handoff", async ({ page }) => {
  await page.getByText("UI Fixture").first().click();

  await expect(page.getByText("Recent sessions")).toBeVisible();
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

  await page.getByTestId("scope-filter").selectOption("project");
  await expect(page.getByTestId("memory-content").first()).toBeVisible();

  // Provenance is a session and a count; evidence content lives locally.
  await expect(page.getByTestId("provenance-session").first()).toBeVisible();
  await expect(page.getByTestId("evidence-count").first()).toContainText("evidence");
});

test("a memory can be deleted from the browser", async ({ page }) => {
  await page.goto(`/projects/${fixture.projectId}/memory`);
  const rows = page.getByTestId("memory-list").getByRole("listitem");
  await expect(rows).toHaveCount(1);

  await page.getByRole("button", { name: "Delete" }).first().click();
  await expect(page.getByText("No memory matches that search.")).toBeVisible();
});

test("tasks and sync status are reachable without a terminal", async ({ page }) => {
  await page.goto(`/projects/${fixture.projectId}`);
  await page.getByTestId("nav-tasks").click();
  await expect(page.getByTestId("task-list")).toContainText("Add rate limiting");
  await page.getByTestId("filter-in_progress").click();
  await expect(page.getByTestId("task-list")).toContainText("Add rate limiting");

  await page.getByTestId("nav-sync").click();
  await expect(page.getByTestId("sync-status")).toContainText("items applied");
});
