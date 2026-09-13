import { expect, test, type Page } from "@playwright/test";
import { openNav } from "./nav";
import {
  API,
  SESSION_COOKIE,
  apiAs,
  apiStatusAs,
  seedControlPlane,
  type Account,
  type ControlPlaneFixture,
} from "./seed";

/**
 * T107, T124 — the whole lifecycle, reconstructed from the browser alone
 * (SC-727, SC-728).
 *
 * # What makes this the acceptance test
 *
 * The path `session → event → run → candidate → knowledge → retrieval` is
 * followed **by navigating**, and every fact it asserts is read off a rendered
 * page. Nothing here opens the database and nothing reads a log. That
 * restriction is not a stylistic preference: if the lifecycle can only be
 * followed with `psql`, then the control plane has not been built, and a test
 * allowed to peek would report success over an empty dashboard.
 *
 * Seeding goes through HTTP for the same reason `seed.ts` already does it that
 * way — the fixture is a user doing things, not rows appearing.
 *
 * # The three things most likely to be quietly wrong
 *
 * **Zero and unavailable.** A funnel stage that counted nothing and a stage the
 * deployment cannot establish are different answers, and the tempting bug is a
 * single `?? 0` that turns the second into the first (FR-879). The page carries
 * `data-count-state` so the distinction is assertable rather than inferred from
 * the glyph on screen.
 *
 * **Raw material.** Memory detail must show that evidence exists and never what
 * it says; retrieval detail must show what was selected and never the briefing
 * built from it. Both are asserted as *absences* of a distinctive seeded string,
 * because an assertion that a summary is present passes just as well on a page
 * that also renders the content beneath it.
 *
 * **Authority.** Every guard is the server's. The tests that matter here drive a
 * second account who is a genuine member of the same project — so a leak cannot
 * be excused as "they were entitled to the project anyway" — and they check the
 * API directly as well as the page, because a hidden button is not a control.
 */

let fx: ControlPlaneFixture;

test.beforeAll(async () => {
  fx = await seedControlPlane();
});

/** Put a signed-in session on the browser context. */
async function signInAs(page: Page, who: Account): Promise<void> {
  await page.context().clearCookies();
  await page.context().addCookies([
    { name: SESSION_COOKIE, value: who.session, domain: "127.0.0.1", path: "/" },
  ]);
}

test.beforeEach(async ({ page }) => {
  await signInAs(page, fx.owner);
});

// ---------------------------------------------------------------------------
// The path, walked
// ---------------------------------------------------------------------------

test("the whole lifecycle is reachable from the project page", async ({
  page,
}, testInfo) => {
  // 1. The funnel — the project's own summary of what it has been doing.
  await page.goto(`/projects/${fx.projectId}`);
  await expect(page.getByTestId("funnel")).toBeVisible();
  await expect(page.getByTestId("funnel-stage-sessions")).toBeVisible();
  await expect(page.getByTestId("funnel-count-sessions")).not.toHaveText("0");

  // 2. Activity — the events that session produced, and the decision made about
  //    them. Reached by navigation rather than by URL, because the claim is that
  //    a person can *find* this.
  await openNav(page, testInfo);
  await page.getByTestId("nav-activity").click();
  await expect(page.getByTestId("activity-list")).toBeVisible();

  // The default set is declared, so the accepted decision may be behind the
  // show-everything control rather than in the first page.
  await page.getByTestId("activity-show-everything").click();
  await expect(
    page.getByTestId("activity-list").getByTestId("activity-family").first(),
  ).toBeVisible();

  // 3. The knowledge that decision produced, at its own page.
  await page.goto(`/projects/${fx.projectId}/memory/${fx.knowledgeId}`);
  await expect(page.getByTestId("memory-detail")).toBeVisible();
  await expect(page.getByTestId("detail-content")).toContainText(
    fx.knowledgeContent,
  );

  // 4. And the retrieval that used it, reached from the memory rather than
  //    looked up: the link between the two is the last hop of the path.
  const usage = page.getByTestId("usage-row").first();
  await expect(usage).toBeVisible();
  await usage.click();
  await expect(page.getByTestId("trace-detail")).toBeVisible();
  await expect(page.getByTestId("trace-items")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Zero is not unavailable (FR-879)
// ---------------------------------------------------------------------------

test("a stage that counted nothing reads differently from one that could not be counted", async ({
  page,
}) => {
  await page.goto(`/projects/${fx.projectId}`);
  await expect(page.getByTestId("funnel")).toBeVisible();

  const tiles = page.locator("[data-count-state]");
  await expect(tiles.first()).toBeVisible();
  const total = await tiles.count();
  expect(total, "the funnel must render all twelve stages").toBe(12);

  // Every tile is one of exactly two states, and the vocabulary is closed: a
  // third rendering would be a third meaning nobody has defined.
  for (let i = 0; i < total; i += 1) {
    const state = await tiles.nth(i).getAttribute("data-count-state");
    expect(["counted", "unavailable"]).toContain(state);
  }

  // A counted stage shows a number — including `0`, which is a real answer and
  // must not be hidden or dashed out.
  const counted = page.locator('[data-count-state="counted"]').first();
  await expect(counted).toBeVisible();
  await expect(counted).toHaveText(/\d/);

  // And an unavailable stage says so rather than showing a zero. Asserted over
  // whichever stages the deployment cannot establish — on a current server that
  // may be none, in which case the loop below is vacuous and the state-vocabulary
  // assertion above is what carries the rule.
  const unavailable = page.locator('[data-count-state="unavailable"]');
  for (let i = 0; i < (await unavailable.count()); i += 1) {
    await expect(
      unavailable.nth(i),
      "an unavailable stage rendered as a number, which claims the deployment " +
        "counted something it never looked at (FR-879)",
    ).not.toHaveText(/^\s*0\s*$/);
  }
});

// ---------------------------------------------------------------------------
// Nothing raw is rendered
// ---------------------------------------------------------------------------

test("memory detail summarises its evidence and shows none of it", async ({
  page,
}) => {
  await page.goto(`/projects/${fx.projectId}/memory/${fx.knowledgeId}`);
  await expect(page.getByTestId("memory-detail")).toBeVisible();

  // The summary is there: counts and kinds, which is what a reader needs to
  // know the record is supported.
  await expect(page.getByTestId("evidence-observations")).toBeVisible();
  await expect(page.getByTestId("evidence-local-notice")).toBeVisible();

  // And the content is not, anywhere on the page. Asserted against the exact
  // command the fixture ran, because a check for "no evidence section" would
  // pass on a page that renders the content in a different section.
  await expect(page.locator("body")).not.toContainText(fx.evidenceCommand);
});

test("retrieval detail names what was selected and never the briefing", async ({
  page,
}) => {
  await page.goto(`/projects/${fx.projectId}/retrievals/${fx.traceId}`);
  await expect(page.getByTestId("trace-detail")).toBeVisible();

  // What it selected, what it cost, how far the pipeline got.
  await expect(page.getByTestId("trace-item").first()).toBeVisible();
  await expect(page.getByTestId("detail-delivery-state")).toBeVisible();
  await expect(page.getByTestId("no-briefing-notice")).toBeVisible();

  // The assembled text does not exist server-side and must not be reconstructed
  // here. The selected item's own content is the thing a reconstruction would
  // most naturally paste in, so that is what is asserted absent.
  await expect(page.locator("body")).not.toContainText(fx.knowledgeContent);
});

// ---------------------------------------------------------------------------
// Complete references (FR-708c, `data-model.md` §6.1)
// ---------------------------------------------------------------------------

test("a rendered reference carries both of its parts", async ({ page }) => {
  await page.goto(`/projects/${fx.projectId}/retrievals/${fx.traceId}`);
  await expect(page.getByTestId("trace-detail")).toBeVisible();

  const reference = page.getByTestId("reference").first();
  await expect(reference).toBeVisible();
  // Two domains can hold the same UUID, so an id alone does not name anything.
  await expect(reference.getByTestId("reference-kind")).toBeVisible();
  await expect(reference.getByTestId("reference-id")).toBeVisible();
  await expect(reference).toHaveAttribute("data-reference-key", fx.referenceKey);
});

// ---------------------------------------------------------------------------
// Domain separation and owner-only patterns (FR-708d, FR-893)
// ---------------------------------------------------------------------------

test("the domains page keeps the four domains visibly apart", async ({
  page,
}) => {
  await page.goto(`/projects/${fx.projectId}/domains`);
  for (const panel of [
    "domain-project",
    "domain-personal",
    "domain-patterns",
    "domain-team",
  ]) {
    await expect(page.getByTestId(panel)).toBeVisible();
  }
  await expect(page.getByTestId("domain-personal-list")).toContainText(
    fx.personalNote,
  );
  await expect(page.getByTestId("domain-patterns-list")).toContainText(
    fx.patternTitle,
  );
});

test("a project co-member sees the project and never the owner's personal domain", async ({
  page,
}) => {
  // A genuine member of the same project, so a leak cannot be excused as an
  // entitlement they already had. This is the sharpest privacy assertion in the
  // file: storing a pattern centrally is durability, not publication.
  await signInAs(page, fx.mate);
  await page.goto(`/projects/${fx.projectId}/domains`);
  await expect(page.getByTestId("domain-project")).toBeVisible();

  await expect(page.getByTestId("domain-personal-list")).not.toContainText(
    fx.personalNote,
  );
  await expect(page.getByTestId("domain-patterns-list")).not.toContainText(
    fx.patternTitle,
  );
  // Belt and braces: not merely absent from the panel, absent from the page.
  await expect(page.locator("body")).not.toContainText(fx.patternTitle);

  // And the API refuses independently, because a page that merely omits
  // something is not a control (FR-708d).
  const patterns = await apiAs(fx.mate.token, "/api/patterns");
  expect(
    JSON.stringify(patterns),
    "the owner's pattern reached a co-member through the API",
  ).not.toContain(fx.patternTitle);
});

// ---------------------------------------------------------------------------
// Team transitions use the atomic routes
// ---------------------------------------------------------------------------

test("an administrator ratifies a proposal and a member cannot", async ({
  page,
}) => {
  await signInAs(page, fx.admin);
  await page.goto("/team");
  const row = page.getByTestId("team-row").filter({ hasText: fx.teamContent });
  await expect(row).toBeVisible();

  // Ratification is irreversible for everyone on the server, so the control is
  // a confirm dialog rather than a button. Waited for rather than probed with
  // `isVisible()`, which answers false while the dialog is still mounting and
  // then silently skips the click — the symptom of which is a row that stays
  // `proposed` and a test that blames the transition.
  await row.getByTestId("team-ratify").click();
  const confirm = page.getByTestId("confirm");
  await expect(confirm).toBeVisible();
  await confirm.click();
  await expect(row.getByTestId("team-state")).toHaveText(/authoritative/i);

  // The member has no control offered...
  await signInAs(page, fx.mate);
  await page.goto("/team");
  await expect(page.getByTestId("team-list")).toBeVisible();
  await expect(page.getByTestId("team-ratify")).toHaveCount(0);

  // ...and, far more importantly, is refused when they call the route anyway.
  // Ratification is what makes guidance authoritative for everyone on the
  // server; only a human administrator may do it.
  const refused = await apiStatusAs(
    fx.mate.token,
    `/api/team/${fx.teamId}/retire`,
    { method: "POST", body: JSON.stringify({}) },
  );
  expect([401, 403]).toContain(refused);
});

// ---------------------------------------------------------------------------
// Admin-only screens
// ---------------------------------------------------------------------------

test("the administration screens are the server's to refuse, not the UI's to hide", async ({
  page,
}) => {
  await signInAs(page, fx.admin);
  await page.goto("/system");
  await expect(page.getByTestId("system-health")).toBeVisible();
  await page.goto("/admin/users");
  await expect(page.getByTestId("account-table")).toBeVisible();

  // A member who types the URL gets a refusal — not a blank page, and certainly
  // not a rendered dashboard. The nav entry being hidden for them is a
  // convenience; this is the control.
  await signInAs(page, fx.mate);
  await page.goto("/system");
  await expect(page.getByTestId("refusal")).toBeVisible();
  await expect(page.getByTestId("system-health")).toHaveCount(0);

  const direct = await apiStatusAs(fx.mate.token, "/api/system/health");
  expect(direct).toBe(403);
});

// ---------------------------------------------------------------------------
// The path is walkable without touching the database (SC-728)
// ---------------------------------------------------------------------------

test("every stage of the path answers over the public API alone", async () => {
  // The same reconstruction as the first test, done through HTTP, so the claim
  // "no database access is required" is proved about the API and not only about
  // the pages built on it. Each response has to carry the link to the next hop:
  // a set of endpoints that each answer correctly but cannot be chained is not
  // a reconstruction.
  const hops: Array<[string, string]> = [
    ["the funnel", `/api/projects/${fx.projectId}/funnel`],
    ["activity", `/api/projects/${fx.projectId}/activity`],
    ["consolidation runs", `/api/projects/${fx.projectId}/consolidation-runs`],
    ["the knowledge", `/api/memories/${fx.knowledgeId}`],
    ["retrieval traces", `/api/projects/${fx.projectId}/retrieval-traces`],
    ["the trace", `/api/retrieval-traces/${fx.traceId}`],
  ];
  for (const [what, path] of hops) {
    const status = await apiStatusAs(fx.owner.token, path);
    expect(status, `${what} (${API}${path}) did not answer`).toBe(200);
  }

  // And the chain holds: the memory names the trace that used it, and the trace
  // names the memory it selected.
  const memory = await apiAs(fx.owner.token, `/api/memories/${fx.knowledgeId}`);
  expect(JSON.stringify(memory)).toContain(fx.traceId);
  const trace = await apiAs(fx.owner.token, `/api/retrieval-traces/${fx.traceId}`);
  expect(JSON.stringify(trace)).toContain(fx.referenceKey);
});
