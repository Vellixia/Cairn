//! The minimal branded page served at `/` when `web/out/` is missing (no Next.js build).
//!
//! In v0.5.0 this doubles as the public landing page for fresh checkouts: install commands
//! (curl / PowerShell), before/after token comparison, and a pointer to the dashboard.
//! The full marketing page ships with the Next.js export and is the canonical landing.
//! This page is intentionally diagnostic-only - never calls any authed endpoint. The only
//! network calls it makes are to the public `/api/health` and `/api/auth/status`.

pub const INDEX_HTML: &str = r###"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Cairn - smart memory for AI agents</title>
<style>
  :root{
    --ink:#0B0F14; --surface:#12181F; --surface2:#1a2129; --slate:#8A94A6;
    --offwhite:#ECEFF4; --ember:#FB923C; --teal:#2DD4BF; --line:#222b35;
  }
  *{box-sizing:border-box}
  body{margin:0;background:radial-gradient(1200px 600px at 70% -10%, #16202b 0%, var(--ink) 55%);
    color:var(--offwhite);font:15px/1.55 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Inter,sans-serif;
    min-height:100vh}
  .wrap{max-width:880px;margin:0 auto;padding:48px 20px 80px}
  header{display:flex;align-items:center;gap:14px;margin-bottom:32px}
  .logo{width:44px;height:44px;flex:0 0 auto}
  h1{font-size:34px;margin:0;letter-spacing:-.02em}
  h2{font-size:18px;margin:0 0 10px;letter-spacing:-.01em}
  .tag{color:var(--slate);margin:6px 0 0;font-size:14px}
  .card{background:var(--surface);border:1px solid var(--line);border-radius:14px;padding:22px;margin-bottom:18px}
  .pill{display:inline-block;border:1px solid var(--line);background:var(--surface2);
    border-radius:999px;padding:4px 10px;font-size:12.5px;color:#b9c2cf;margin-right:6px}
  .cta{display:inline-block;margin-top:14px;background:var(--ember);color:#1a1206;border:0;
    border-radius:9px;padding:11px 20px;font-weight:700;text-decoration:none;font-size:14px}
  .cta.ghost{background:transparent;color:var(--offwhite);border:1px solid var(--line);margin-left:8px}
  pre.codeblock{background:var(--surface2);border:1px solid var(--line);border-radius:9px;
    padding:12px 14px;font:12.5px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;
    color:#cdd5e0;overflow:auto;margin:8px 0 0;white-space:pre}
  .stat{display:flex;justify-content:space-between;padding:4px 0;font-size:13.5px;
    border-bottom:1px dashed var(--line)}
  .stat:last-child{border-bottom:0}
  .stat b{color:var(--teal);font-variant-numeric:tabular-nums;font-weight:600}
  table.compare{width:100%;border-collapse:collapse;font-size:13.5px;margin-top:10px}
  table.compare th,table.compare td{text-align:left;padding:8px 10px;border-bottom:1px solid var(--line)}
  table.compare th{color:var(--slate);font-weight:500;font-size:11.5px;text-transform:uppercase;letter-spacing:.04em}
  table.compare td b{color:var(--teal)}
  .installs{display:grid;grid-template-columns:1fr 1fr 1fr;gap:10px;margin-top:12px}
  .installs .inst{background:var(--surface2);border:1px solid var(--line);border-radius:9px;padding:12px}
  .installs .inst h3{margin:0 0 6px;font-size:12px;color:var(--slate);text-transform:uppercase;letter-spacing:.06em;font-weight:500}
  footer{margin-top:36px;color:var(--slate);font-size:12.5px}
  a{color:var(--teal)}
</style>
</head>
<body>
<div class="wrap">
  <header>
    <svg class="logo" viewBox="0 0 48 48" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <ellipse cx="24" cy="38" rx="15" ry="2.6" fill="#000000" opacity="0.28"/>
      <ellipse cx="24" cy="34" rx="15" ry="5" fill="#6B7689"/>
      <ellipse cx="22.5" cy="28" rx="12" ry="4.2" fill="#8A94A6"/>
      <ellipse cx="25.5" cy="23" rx="9" ry="3.5" fill="#AEB8C6"/>
      <ellipse cx="24" cy="18" rx="6" ry="3.3" fill="#FB923C"/>
    </svg>
    <div>
      <h1>Cairn</h1>
      <p class="tag">Smart memory for any AI agent - self-hosted - Apache-2.0</p>
    </div>
  </header>

  <div class="card">
    <h2>Make any model remember what matters</h2>
    <p style="color:var(--slate);margin:0 0 12px;font-size:14px">
      Cairn sits between your agent and your code. It remembers, recalls, and verifies - and
      every decision is signed so the full-fidelity original is always one click away.
    </p>
    <div class="installs">
      <div class="inst">
        <h3>macOS / Linux</h3>
        <pre class="codeblock">curl -fsSL https://cairn.sh/install.sh | sh</pre>
      </div>
      <div class="inst">
        <h3>Docker</h3>
        <pre class="codeblock">docker compose up -d</pre>
      </div>
      <div class="inst">
        <h3>Windows (PowerShell)</h3>
        <pre class="codeblock">iwr cairn.sh/install.ps1 | iex</pre>
      </div>
    </div>
    <a id="primaryCta" class="cta" href="/login">Open dashboard</a>
    <a class="cta ghost" href="/api/health" target="_blank" rel="noopener">/api/health</a>
  </div>

  <div class="card">
    <h2>Before / after - token savings on a real repo</h2>
    <table class="compare">
      <thead><tr><th>Scenario</th><th>Without Cairn</th><th>With Cairn</th><th>Reduction</th></tr></thead>
      <tbody>
        <tr><td>Read 1 source file (full)</td><td>~3,200 tokens</td><td>~210 tokens</td><td><b>'93%</b></td></tr>
        <tr><td>Recall 10 relevant memories</td><td>~12,000 tokens</td><td>~1,800 tokens</td><td><b>'85%</b></td></tr>
        <tr><td>Assemble context (8k budget)</td><td>~8,000 tokens (no ranking)</td><td>~5,300 tokens (edge-ordered)</td><td><b>'34%</b></td></tr>
        <tr><td>Verify an edit post-write</td><td>silent corruption in 25% of edits</td><td>flagged before commit</td><td><b>trust</b></td></tr>
      </tbody>
    </table>
    <p style="color:var(--slate);margin:12px 0 0;font-size:12px">
      Numbers from <code>cairn bench</code> on a 200 KiB Rust crate. Reproducible on your
      own code in &lt;5 s.
    </p>
  </div>

  <div class="card">
    <h2>Server status</h2>
    <div class="stat"><span>Health</span><b id="health">checking...</b></div>
    <div class="stat"><span>Version</span><b id="version">...</b></div>
    <div class="stat"><span>Admin</span><b id="admin">checking...</b></div>
    <div class="stat"><span>Next step</span><b id="next">...</b></div>
  </div>

  <footer>:: Cairn - Apache-2.0 - every traveler adds a stone.</footer>
</div>

<script>
const $ = (id) => document.getElementById(id);
async function probe(){
  let healthy = false;
  try {
    const h = await (await fetch('/api/health')).json();
    $('health').textContent = h.status;
    $('version').textContent = 'v' + h.version;
    healthy = h.status === 'ok';
  } catch (e) {
    $('health').textContent = 'unreachable';
  }
  if (!healthy) {
    $('admin').textContent = 'unknown';
    $('next').textContent = 'start the server';
    $('primaryCta').textContent = 'Start server';
    $('primaryCta').removeAttribute('href');
    return;
  }
  try {
    const a = await (await fetch('/api/auth/status')).json();
    $('admin').textContent = a.admin_exists ? 'configured' : 'not configured';
    if (a.setup_required) {
      $('next').textContent = 'first-run setup';
      $('primaryCta').textContent = 'Create admin';
      $('primaryCta').href = '/setup/wizard';
    } else {
      $('next').textContent = 'sign in';
      $('primaryCta').textContent = 'Sign in';
      $('primaryCta').href = '/login';
    }
  } catch (e) {
    $('admin').textContent = 'unknown';
    $('next').textContent = 'sign in';
  }
}
probe();
</script>
</body>
</html>"###;
