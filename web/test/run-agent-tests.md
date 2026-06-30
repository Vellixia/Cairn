# Drive the cairn dashboard flow tests

You are running the dashboard flow suite. The cairn stack must be live at `http://127.0.0.1:7777` (admin credentials `admin / AuditPass2026!`).

## What you do

For each of the 13 flows in `flows.md`, drive the dashboard via the `chrome-devtools` MCP server, assert on real DOM state, screenshot, and either mark the flow PASS or write a finding to `findings/`.

## Step-by-step

1. Read `web/test/flows.md` end-to-end. Note the 13 flows and the conventions.
2. Open a fresh Chrome page with `new_page("http://127.0.0.1:7777/")`. This is your canvas for the whole run.
3. For each flow `NN`, in order:
   a. Read the flow's section in `flows.md` carefully.
   b. Run each step using `navigate_page`, `fill`, `click`, `press_key`, `wait_for`, etc.
   c. After every interactive step, call `take_snapshot` to get the accessibility tree. Verify the asserted refs/texts are present.
   d. Save a screenshot via `take_screenshot` to `web/test/screenshots/<NN>-<flow>/<step>.png`.
   e. At the end, call `list_console_messages(types=["error"])`. If there is an uncaught JS error from the dashboard's own code, write a finding.
   f. Either write a one-line summary of the flow result into your final reply, or write a finding file.

## Non-negotiable rules

- **A step that times out, returns no snapshot, or returns an identical-looking screenshot to the previous step is a failure.** Write a finding. Do not "PASS" the flow.
- **Don't guess at selectors.** Use `take_snapshot` to find the right ref by text content.
- **Don't skip the assertion because it's "probably fine".** If you can't confirm, write a finding.
- **Two findings from the prior harness are confirmed real bugs:** `/memory/architecture` Next.js crash and `/mobile` HTML-instead-of-JSON. Both should appear as findings again — confirm by snapshotting them and checking for the error text.
- **Save screenshots even when the flow fails** — the screenshot is the most useful evidence in the finding.

## Final deliverable

When the run is complete, write a one-paragraph summary at the top of `findings/SUMMARY.md`:

- Total flows driven
- Total PASS / FAIL
- One line per finding: file path + one-sentence description

Then list, in your final reply, the path to every finding you wrote.

## What this is NOT

- This is not a unit-test runner. Don't run `cargo test`.
- This is not a CI step. The cairn dashboard flows are local-only because they need a running stack and a real browser.
- This is not a regression smoke. Every flow is allowed to fail; that's the point.