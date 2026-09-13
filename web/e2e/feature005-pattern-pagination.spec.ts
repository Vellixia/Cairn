import { expect, test, type Page } from "@playwright/test";
import {
  SESSION_COOKIE,
  apiAs,
  createProject,
  newToken,
  registerAndLogin,
  type Account,
} from "./seed";

/**
 * FR-895 — the Domains pattern panel is bounded **and** paginated, in a
 * browser, across the page boundary.
 *
 * # Why this needs a browser test at all
 *
 * `/api/patterns` has two modes and the difference between them is the whole
 * requirement. A caller that asks for no bound is the daemon refilling its
 * pattern cache and gets **every** pattern, because a truncated refill would
 * silently shorten a durability guarantee. A caller that asks for a bound is a
 * screen, and FR-895 says a screen must page. The server-side test
 * (`feature005_patterns::the_pattern_list_pages_over_a_stable_order_and_still_answers_in_full`)
 * proves the endpoint honours both. It cannot prove the *panel* uses the
 * paginated one — a UI that quietly called the unbounded mode and rendered
 * every row would pass every server test in the repository.
 *
 * So this test asserts the boundary from the outside: 30 patterns exist, the
 * first view shows 25, and the thirty-first through fortieth row only appear
 * because a person asked for them.
 *
 * # Why the assertions are set-based rather than positional
 *
 * A bounded read pages over `pattern_id`, which is `UUIDv5(owner ‖
 * content_key)` — stable, which is the point, and therefore unrelated to
 * creation order. Nothing here may assert *which* patterns land on the first
 * page. What it can assert, and what "no skips, no duplicates" actually means,
 * is that the union of the two pages is exactly the seeded set and that no
 * title is rendered twice.
 *
 * # The isolation assertion is not decoration
 *
 * A second owner's pattern is seeded and must never appear. Paging is a place
 * where a scoping mistake hides especially well: the first page can be
 * correctly filtered while a cursor-resumed query drops the owner predicate,
 * and only a fixture with someone else's rows in the same table would notice.
 */

/** What the panel asks for, and therefore where the boundary sits. */
const PAGE = 25;
/** Enough to make the boundary obvious: one full page and a short one. */
const TOTAL = 30;

interface Fixture {
  owner: Account;
  /** A second account with a pattern of its own, which must stay invisible. */
  stranger: Account;
  strangerPatternTitle: string;
  projectId: string;
  /** Every title this owner promoted, in creation order. */
  titles: string[];
}

let fx: Fixture;

async function account(label: string): Promise<Account> {
  const email = `${label}-${Date.now()}-${Math.floor(Math.random() * 1e6)}@example.test`;
  const password = "hunter2hunter2";
  const displayName = `FR895 ${label}`;
  const session = await registerAndLogin(email, password, displayName);
  const token = await newToken(session, `${label}-fr895`);
  const me = await apiAs(token, "/api/auth/me");
  return { email, password, displayName, session, token, id: me.id as string };
}

/**
 * Promote one pattern through the real route.
 *
 * The content is deliberately about nothing: a promotion naming any of the
 * caller's own projects is refused by the global content screen (FR-822), and
 * a fixture that tripped it would fail as a seeding error rather than as
 * anything about pagination.
 *
 * **It also avoids the substrings `com` and `git`.** The route's own identity
 * set (`cairn-server/src/commands.rs::all_identities_for`) splits a remote on
 * `.` as well as `/:@` and applies no structural-token filter, so a project on
 * `github.com` contributes `github` *and* `com` as project identities — and
 * `names_a_project` folds separators and asks `contains`, so any text holding
 * `com` is refused. That is why this fixture says "checked against" rather
 * than "compared": the first spelling seeds, the second is refused as
 * `project_identifying`. It is a real over-refusal, recorded here because a
 * later reader will otherwise rediscover it as an inexplicable seeding
 * failure — `global.rs::remote_tokens`, the other implementation of the same
 * screen, filters exactly these tokens for exactly this reason.
 */
async function promote(token: string, title: string): Promise<void> {
  await apiAs(token, "/api/patterns", {
    method: "POST",
    body: JSON.stringify({
      title,
      problem: `A retry loop stops early when the clock moves backwards: ${title}.`,
      root_cause: `The deadline is checked against a stale reading: ${title}.`,
      approach: `Refresh the deadline inside the loop and read it monotonically: ${title}.`,
    }),
  });
}

test.beforeAll(async () => {
  const run = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const owner = await account("fr895-owner");
  const stranger = await account("fr895-stranger");

  // Patterns are owner-scoped, not project-scoped; the project exists only
  // because the Domains panel lives under a project's route.
  const projectId = await createProject(
    owner.token,
    `FR895 Pagination ${run}`,
    `github.com/example/fr895-${run}`,
  );

  // Distinctive and individually identifiable, so "which rows are on screen"
  // is answerable exactly rather than by counting.
  //
  // **Nothing here may repeat the run token.** It appears in this owner's
  // project name and remote, and the global content screen refuses a promotion
  // that names one of the caller's own projects (FR-822) — sharing the token
  // made every seeding call fail as `project_identifying`. Uniqueness across
  // the two Playwright projects is not needed anyway: patterns are owner-scoped
  // and each project seeds its own owner.
  const titles = Array.from(
    { length: TOTAL },
    (_, i) => `Marmot ledger drift ${String(i).padStart(2, "0")}`,
  );
  for (const title of titles) {
    await promote(owner.token, title);
  }

  const strangerPatternTitle = "Heron cache thaw before the first read";
  await promote(stranger.token, strangerPatternTitle);

  fx = { owner, stranger, strangerPatternTitle, projectId, titles };
});

async function signInAs(page: Page, who: Account): Promise<void> {
  await page.context().clearCookies();
  await page.context().addCookies([
    { name: SESSION_COOKIE, value: who.session, domain: "127.0.0.1", path: "/" },
  ]);
}

/** The titles currently rendered in the pattern panel, in DOM order. */
async function renderedTitles(page: Page): Promise<string[]> {
  return page
    .getByTestId("domain-pattern-row")
    .locator("p.font-medium")
    .allTextContents();
}

test("the pattern panel shows one bounded page and reaches the rest through Show more", async ({
  page,
}) => {
  await signInAs(page, fx.owner);
  await page.goto(`/projects/${fx.projectId}/domains`);
  await expect(page.getByTestId("domain-patterns")).toBeVisible();

  // 1. The initial view is bounded — one page, not the owner's whole corpus.
  //    This is the assertion a UI calling the daemon's unbounded refill mode
  //    would fail, and the reason the test exists.
  const rows = page.getByTestId("domain-pattern-row");
  await expect(rows).toHaveCount(PAGE);
  const first = await renderedTitles(page);
  expect(first).toHaveLength(PAGE);

  // 2. Show more is offered, and 3. it tells the truth about what is left:
  //    25 of 30, not "25" alone and not a guess.
  const more = page.getByTestId("domain-patterns-more");
  await expect(more).toBeVisible();
  await expect(more).toHaveText(`Show more (${PAGE} of ${TOTAL})`);

  // 4. Asking fetches the next page and renders it.
  await more.click();
  await expect(rows).toHaveCount(TOTAL);
  const both = await renderedTitles(page);

  // 5. Nothing is duplicated across the boundary — the failure a cursor that
  //    resumes from the wrong key produces.
  expect(new Set(both).size).toBe(both.length);

  // 6. Nothing is skipped — the *other* failure of the same wrong cursor, and
  //    the one an "it rendered 30 rows" assertion alone would miss.
  expect([...both].sort()).toEqual([...fx.titles].sort());

  // The second page's rows are genuinely on screen, named individually rather
  // than inferred from the count.
  const second = both.filter((t) => !first.includes(t));
  expect(second).toHaveLength(TOTAL - PAGE);
  for (const title of second) {
    await expect(
      page.getByTestId("domain-pattern-row").filter({ hasText: title }),
    ).toHaveCount(1);
  }

  // 8. And the control stops offering what does not exist: the last page is
  //    short, so there is nothing to resume from and no button to press.
  await expect(more).toHaveCount(0);
});

test("paging never reaches another owner's patterns", async ({ page }) => {
  await signInAs(page, fx.owner);
  await page.goto(`/projects/${fx.projectId}/domains`);
  await expect(page.getByTestId("domain-patterns")).toBeVisible();

  const more = page.getByTestId("domain-patterns-more");
  await expect(more).toBeVisible();
  await more.click();
  await expect(page.getByTestId("domain-pattern-row")).toHaveCount(TOTAL);

  // Not merely absent from the panel — absent from the page, on the resumed
  // page as well as the first, which is where a dropped owner predicate would
  // show itself.
  await expect(page.locator("body")).not.toContainText(fx.strangerPatternTitle);

  // And the API refuses independently of what the panel chose to render: a
  // page that omits something is not a control (FR-708d). Asked with the same
  // bound the panel uses, and again with no bound at all — the daemon's
  // complete-refill mode, which must be owner-scoped too.
  const bounded = await apiAs(fx.owner.token, `/api/patterns?limit=${PAGE}`);
  expect(JSON.stringify(bounded)).not.toContain(fx.strangerPatternTitle);
  const unbounded = await apiAs(fx.owner.token, "/api/patterns");
  expect(JSON.stringify(unbounded)).not.toContain(fx.strangerPatternTitle);

  // The unbounded mode still answers in full, which is the daemon's cache
  // refill contract and the thing the paginated panel must not have cost it.
  expect(unbounded.returned).toBe(TOTAL);
  expect(unbounded.total).toBe(TOTAL);
  expect(unbounded.cursor).toBeNull();
});
