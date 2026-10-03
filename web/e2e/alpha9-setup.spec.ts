import { expect, test } from "@playwright/test";
import { registerAndLogin, SESSION_COOKIE } from "./seed";

// Real account/API; transport interception only exercises pending/outage UI.
test.beforeEach(async ({ context }) => {
  const session = await registerAndLogin(
    `alpha9-${crypto.randomUUID()}@example.test`,
    "hunter2hunter2",
    "Alpha9 developer",
  );
  await context.addCookies([{
    name: SESSION_COOKIE,
    value: session,
    url: process.env.CAIRN_WEB_URL ?? "http://127.0.0.1:3100",
  }]);
});

test("browser creates setup-ready remote and reports invalid input without losing it", async ({ page }) => {
  await page.goto("/settings");
  await page.getByLabel("Project name", { exact: true }).fill("Fresh repository");
  await page.getByLabel("Repository remote", { exact: true }).fill("not-a-remote");
  await page.getByRole("button", { name: "Create project", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: "remote" })).toBeVisible();
  await expect(page.getByLabel("Project name", { exact: true })).toHaveValue("Fresh repository");
  await expect(page.getByLabel("Repository remote", { exact: true })).toHaveValue("not-a-remote");
  const remote = `git@github.com:example/alpha9-${crypto.randomUUID()}.git`;
  await page.getByLabel("Repository remote", { exact: true }).fill(remote);
  const created = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/projects" && response.request().method() === "POST" && response.ok());
  await page.getByRole("button", { name: "Create project", exact: true }).click();
  const project = await (await created).json();
  expect(project.repository_remote).toBe(remote.replace("git@github.com:", "github.com/").replace(/\.git$/, ""));
  await expect(page.getByRole("status").filter({ hasText: "Project created" })).toBeVisible();
  await expect(page.getByTestId("project-members")).toContainText("Fresh repository");
  await expect(page.getByTestId("project-members")).toContainText("Alpha9 developer");
  await page.reload();
  await expect(page.getByTestId("settings-projects")).toContainText("Fresh repository");
});

test("pending project submission cannot duplicate and outage retains input", async ({ page }) => {
  let submissions = 0;
  let release!: () => void;
  const pending = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/api/projects", async (route) => {
    if (route.request().method() !== "POST") return route.continue();
    submissions += 1;
    await pending;
    await route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: { code: "unavailable", message: "Server unavailable. Retry when connection returns." } }) });
  });
  await page.goto("/settings");
  await page.getByLabel("Project name", { exact: true }).fill("Keep my input");
  await page.getByLabel("Repository remote", { exact: true }).fill("https://github.com/example/retained.git");
  await page.getByRole("button", { name: "Create project", exact: true }).click();
  await expect(page.getByRole("button", { name: "Creating…", exact: true })).toBeDisabled();
  await expect.poll(() => submissions).toBe(1);
  release();
  await expect(page.getByRole("alert").filter({ hasText: "Server unavailable" })).toBeVisible();
  await expect(page.getByLabel("Project name", { exact: true })).toHaveValue("Keep my input");
  await expect(page.getByLabel("Repository remote", { exact: true })).toHaveValue("https://github.com/example/retained.git");
  await expect(page.getByRole("button", { name: "Create project", exact: true })).toBeEnabled();
});

test("privacy outage and member refusal have actionable feedback", async ({ page }) => {
  let failPrivacy = true;
  await page.route("**/api/privacy-policy", async (route) => {
    if (!failPrivacy) return route.continue();
    await route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: { code: "unavailable", message: "Privacy policy unavailable. Retry." } }) });
  });
  await page.goto("/settings");
  const privacy = page.getByTestId("privacy-policy");
  await expect(privacy.getByRole("alert")).toContainText("Privacy policy unavailable");
  await expect(privacy).not.toContainText("Loading privacy policy");
  failPrivacy = false;
  await privacy.getByRole("button", { name: "Retry privacy policy" }).click();
  await expect(privacy).toContainText("Raw observations are not stored");

  await page.getByLabel("Project name", { exact: true }).fill("Member feedback");
  await page.getByLabel("Repository remote", { exact: true }).fill(`https://github.com/example/members-${crypto.randomUUID()}.git`);
  await page.getByRole("button", { name: "Create project", exact: true }).click();
  const members = page.getByTestId("project-members");
  const missingAccount = crypto.randomUUID();
  await members.getByLabel("Member user ID", { exact: true }).fill(missingAccount);
  await members.getByRole("button", { name: "Add member", exact: true }).click();
  await expect(members.getByRole("alert")).toBeVisible();
  await expect(members.getByLabel("Member user ID", { exact: true })).toHaveValue(missingAccount);
});
