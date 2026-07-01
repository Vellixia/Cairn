# /memory - Run rust-ext-1 step 02

## Expected
Memory hub renders tabs (Wakeup, Recall, Graph, Compression Lab, Savings, Architecture, Activity) with the default Wakeup tab content listing memories.

## Observed
- URL: http://127.0.0.1:7777/memory?cb=run-rust-ext-1-02
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Memory & Context", sub-nav of 7 tabs, Wakeup section with 2 memories listed (DRIFT_TRIGGER_CAIRN_TEST and cairn v0.6.1 fresh install docker test)
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: nav with Wakeup/Recall/Graph/Compression Lab/Savings/Architecture/Activity; heading "Wakeup"; 2 fact entries with tier+importance+confidence

## Verdict
PASS

## Notes
Screenshot skipped (timeout on prior step).
