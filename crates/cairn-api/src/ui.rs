//! The minimal branded page served at `/`. This is a placeholder control plane: it proves the
//! engine works in a browser (live health/stats, remember, recall) until the full Next.js app is
//! embedded. Kept dependency-free (inline CSS/JS) so the single binary is truly self-contained.

pub const INDEX_HTML: &str = r###"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Cairn — context & reliability for AI agents</title>
<style>
  :root{
    --ink:#0B0F14; --surface:#12181F; --surface2:#1a2129; --slate:#8A94A6;
    --offwhite:#ECEFF4; --ember:#FB923C; --teal:#2DD4BF; --line:#222b35;
  }
  *{box-sizing:border-box}
  body{margin:0;background:radial-gradient(1200px 600px at 70% -10%, #16202b 0%, var(--ink) 55%);
    color:var(--offwhite);font:15px/1.55 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Inter,sans-serif;
    min-height:100vh}
  .wrap{max-width:860px;margin:0 auto;padding:48px 20px 80px}
  header{display:flex;align-items:center;gap:14px;margin-bottom:8px}
  .logo{width:40px;height:40px;flex:0 0 auto}
  h1{font-size:30px;margin:0;letter-spacing:-.02em}
  .tag{color:var(--slate);margin:2px 0 0;font-size:14px}
  .hero{margin:30px 0 26px}
  .hero p{font-size:18px;color:#cdd5e0;margin:.4em 0}
  .accent{color:var(--ember)}
  .pillars{display:flex;flex-wrap:wrap;gap:8px;margin:18px 0 6px}
  .pill{border:1px solid var(--line);background:var(--surface);border-radius:999px;
    padding:5px 12px;font-size:12.5px;color:#b9c2cf}
  .grid{display:grid;grid-template-columns:1fr 1fr;gap:16px;margin-top:26px}
  @media(max-width:680px){.grid{grid-template-columns:1fr}}
  .card{background:var(--surface);border:1px solid var(--line);border-radius:14px;padding:18px}
  .card h2{font-size:13px;text-transform:uppercase;letter-spacing:.08em;color:var(--slate);margin:0 0 12px}
  .stat{display:flex;justify-content:space-between;padding:6px 0;border-bottom:1px dashed var(--line)}
  .stat:last-child{border-bottom:0}
  .stat b{color:var(--teal);font-variant-numeric:tabular-nums}
  textarea,input{width:100%;background:var(--surface2);border:1px solid var(--line);color:var(--offwhite);
    border-radius:9px;padding:10px;font:inherit;resize:vertical}
  button{margin-top:10px;background:var(--ember);color:#1a1206;border:0;border-radius:9px;
    padding:9px 16px;font-weight:600;cursor:pointer}
  button.ghost{background:transparent;color:var(--offwhite);border:1px solid var(--line)}
  .results{margin-top:12px;display:flex;flex-direction:column;gap:8px}
  .mem{background:var(--surface2);border:1px solid var(--line);border-radius:9px;padding:9px 11px;font-size:13.5px}
  .mem .meta{color:var(--slate);font-size:11.5px;margin-top:4px}
  .score{color:var(--ember);font-variant-numeric:tabular-nums}
  footer{margin-top:40px;color:var(--slate);font-size:13px}
  a{color:var(--teal)}
  code{background:var(--surface2);padding:2px 6px;border-radius:6px;color:#cdd5e0}
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
      <p class="tag">The open-source context &amp; reliability layer for AI agents</p>
    </div>
  </header>

  <section class="hero">
    <p><strong>Make any model smart.</strong> Remember everything · feed less, not more · stay
       reliable on long tasks · get smarter together — self-hosted, one Rust binary,
       <span class="accent">with no context ever lost</span>.</p>
    <div class="pillars">
      <span class="pill">Remember</span>
      <span class="pill">Compress · no loss</span>
      <span class="pill">Assemble lean context</span>
      <span class="pill">Stay reliable</span>
      <span class="pill">Smarter together</span>
    </div>
  </section>

  <div class="grid">
    <div class="card">
      <h2>Server</h2>
      <div class="stat"><span>Status</span><b id="status">…</b></div>
      <div class="stat"><span>Version</span><b id="version">…</b></div>
      <div class="stat"><span>Memories stored</span><b id="memcount">…</b></div>
      <p style="color:var(--slate);font-size:12.5px;margin:12px 0 0">
        API: <code>/api/health</code> · <code>/api/memory/recall?q=…</code> ·
        <code>/api/context/read?path=…</code>
      </p>
    </div>

    <div class="card">
      <h2>Remember something</h2>
      <textarea id="rememberText" rows="3" placeholder="e.g. We chose SQLite + a content-hash blob store so compression stays lossless."></textarea>
      <button onclick="remember()">Remember</button>
      <div id="rememberOut" style="color:var(--teal);font-size:12.5px;margin-top:8px"></div>
    </div>
  </div>

  <div class="card" style="margin-top:16px">
    <h2>Recall</h2>
    <input id="recallQuery" placeholder="search your memory… e.g. storage decision" onkeydown="if(event.key==='Enter')recall()" />
    <button class="ghost" onclick="recall()">Recall</button>
    <div class="results" id="recallOut"></div>
  </div>

  <footer>
    🪨 Early development · this page is a placeholder control plane; the full dashboard is coming.
    &nbsp;·&nbsp; <a href="https://github.com/Vellixia/cairn">GitHub</a>
  </footer>
</div>

<script>
const $ = (id) => document.getElementById(id);
async function refresh(){
  try{
    const h = await (await fetch('/api/health')).json();
    $('status').textContent = h.status; $('version').textContent = 'v'+h.version;
    const s = await (await fetch('/api/stats')).json();
    $('memcount').textContent = s.memories;
  }catch(e){ $('status').textContent = 'offline'; }
}
async function remember(){
  const content = $('rememberText').value.trim();
  if(!content) return;
  const r = await fetch('/api/memory', {method:'POST',headers:{'content-type':'application/json'},
    body: JSON.stringify({content})});
  const m = await r.json();
  $('rememberOut').textContent = 'stored ('+m.kind+'/'+m.tier+') · id '+m.id.slice(0,8);
  $('rememberText').value=''; refresh();
}
async function recall(){
  const q = $('recallQuery').value.trim();
  const r = await fetch('/api/memory/recall?limit=8&q='+encodeURIComponent(q));
  const hits = await r.json();
  $('recallOut').innerHTML = hits.length ? hits.map(h =>
    `<div class="mem">${escapeHtml(h.memory.content)}
       <div class="meta"><span class="score">${h.score.toFixed(2)}</span> ·
       ${h.memory.kind} · ${h.memory.tier} · ${new Date(h.memory.created_at).toLocaleString()}</div></div>`
  ).join('') : '<div style="color:var(--slate);font-size:13px">no matches yet</div>';
}
function escapeHtml(s){return s.replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));}
refresh();
</script>
</body>
</html>"###;
