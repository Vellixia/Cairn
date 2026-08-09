import type { Page, TestInfo } from "@playwright/test";

/**
 * Reveal the sidebar before clicking something in it.
 *
 * Below the mobile breakpoint the sidebar is a sheet, closed until asked for,
 * so a test that clicks a nav item straight away waits for an element nobody
 * can see. On desktop this does nothing.
 */
export async function openNav(page: Page, testInfo: TestInfo): Promise<void> {
  if (testInfo.project.name !== "mobile-chromium") return;
  const trigger = page.getByRole("button", { name: "Toggle Sidebar" });
  await trigger.click();
  await page.getByTestId("nav-projects").waitFor({ state: "visible" });
}
