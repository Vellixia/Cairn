import Link from "next/link";
import Logo from "@/components/Logo";

/**
 * Cairn's public landing page (v0.5.0 Sprint 17). Renders at `/` when the
 * Next.js export is built. The cairn-server's static fallback (`INDEX_HTML`)
 * renders the equivalent minimal page when `web/out` is missing.
 *
 * Sections:
 *   1. Hero — one-sentence value prop + install commands.
 *   2. Demo GIF placeholder — see `/assets/demo-placeholder.svg`.
 *   3. Token savings — quick "before / after" table that mirrors BENCHMARKS.md.
 *   4. Comparison — honest table comparing Cairn to memory-only / RAG-only tools.
 *   5. Install paths — curl, Homebrew, Docker, cargo install, npm-free.
 *   6. Trust signals — Apache-2.0, no telemetry, self-hosted, signed releases.
 *   7. Footer — links to docs/PLAN, docs/ARCHITECTURE, docs/SECURITY, etc.
 *
 * Design constraint: no auth check on this page (it's public). The Cairn API
 * is only probed from the server side via the `cairn.sh/landing` endpoint if
 * available; otherwise we render statically.
 */
export default function LandingPage() {
  return (
    <main className="min-h-screen bg-gradient-to-b from-[#0B0F14] via-[#12181F] to-[#0B0F14] text-[#ECEFF4]">
      <div className="mx-auto max-w-5xl px-6 py-16 sm:py-24">
        {/* Hero */}
        <header className="mb-20">
          <div className="flex items-center gap-4 mb-8">
            <Logo />
            <span className="text-2xl font-semibold tracking-tight">Cairn</span>
          </div>
          <h1 className="text-5xl sm:text-6xl font-semibold tracking-tight leading-[1.05] mb-6">
            Make any AI agent remember what matters.
          </h1>
          <p className="text-xl text-[#8A94A6] max-w-2xl mb-10">
            Cairn is a self-hosted memory + reliability layer that sits between
            your AI agent and your code. It remembers decisions, recalls them
            when relevant, and verifies every edit.{" "}
            <span className="text-[#FB923C]">Single binary. Apache-2.0.</span>
          </p>
          <div className="flex flex-wrap gap-3 mb-8">
            <Link
              href="/dashboard"
              className="inline-flex items-center gap-2 rounded-md bg-[#FB923C] px-5 py-3 text-sm font-semibold text-[#1a1206] hover:opacity-90 transition"
            >
              Open dashboard →
            </Link>
            <a
              href="https://github.com/Vellixia/Cairn#quick-start"
              className="inline-flex items-center gap-2 rounded-md border border-[#222b35] bg-[#12181F] px-5 py-3 text-sm font-semibold text-[#ECEFF4] hover:bg-[#1a2129] transition"
            >
              Install
            </a>
            <a
              href="https://github.com/Vellixia/Cairn/blob/main/docs/BENCHMARKS.md"
              className="inline-flex items-center gap-2 rounded-md border border-[#222b35] bg-transparent px-5 py-3 text-sm font-semibold text-[#ECEFF4] hover:bg-[#12181F] transition"
            >
              Benchmarks
            </a>
          </div>
          <p className="text-xs text-[#8A94A6]">
            Works with Claude Code · OpenCode · Cursor · VS Code · Windsurf · any MCP agent
          </p>
        </header>

        {/* Demo placeholder */}
        <DemoPlaceholder />

        {/* Token savings */}
        <section className="mb-20">
          <h2 className="text-3xl font-semibold tracking-tight mb-3">
            Token savings — measured, not promised
          </h2>
          <p className="text-[#8A94A6] mb-6 max-w-2xl">
            Every Cairn release publishes reproducible benchmarks. The numbers
            below are from <code>cairn-cli bench</code> running on Cairn&apos;s
            own <code>crates/</code>. Run the same command on your own code in
            under 5 seconds — see <Link href="/docs/BENCHMARKS.md" className="text-[#2DD4BF]">BENCHMARKS.md</Link>.
          </p>
          <div className="rounded-lg border border-[#222b35] bg-[#12181F] overflow-hidden">
            <table className="w-full text-sm">
              <thead className="bg-[#1a2129] text-[#8A94A6] text-left">
                <tr>
                  <th className="py-3 px-4 font-medium">Scenario</th>
                  <th className="py-3 px-4 font-medium text-right">Without Cairn</th>
                  <th className="py-3 px-4 font-medium text-right">With Cairn</th>
                  <th className="py-3 px-4 font-medium text-right">Reduction</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[#222b35]">
                <Row label="AST outline read (1 file)" before="~3,200 tok" after="~210 tok" pct="93%" />
                <Row label="Recall 10 relevant memories" before="~12,000 tok" after="~1,800 tok" pct="85%" />
                <Row label="Assemble context (8k budget)" before="~8,000 tok (no ranking)" after="~5,300 tok (edge-ordered)" pct="34%" />
                <Row label="Re-read unchanged file" before="~6,506 tok" after="~19 tok (handle only)" pct="99.7%" />
                <Row label="Compress verbose test log" before="153 lines" after="1 line (full recoverable)" pct="99%" />
              </tbody>
            </table>
          </div>
        </section>

        {/* Honest comparison */}
        <section className="mb-20">
          <h2 className="text-3xl font-semibold tracking-tight mb-3">
            Honest comparison
          </h2>
          <p className="text-[#8A94A6] mb-6 max-w-2xl">
            We don&apos;t claim to be everything to everyone. Here&apos;s how
            Cairn compares to a few categories of related tools — and where it
            isn&apos;t the right fit.
          </p>
          <div className="rounded-lg border border-[#222b35] bg-[#12181F] overflow-hidden">
            <table className="w-full text-sm">
              <thead className="bg-[#1a2129] text-[#8A94A6] text-left">
                <tr>
                  <th className="py-3 px-4 font-medium">Capability</th>
                  <th className="py-3 px-4 font-medium">Cairn</th>
                  <th className="py-3 px-4 font-medium">Memory-only (mem0, agentmemory)</th>
                  <th className="py-3 px-4 font-medium">RAG-only (vector DB)</th>
                  <th className="py-3 px-4 font-medium">Hosted context (Zep, etc.)</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[#222b35]">
                <CompareRow
                  feature="Cross-session memory"
                  cairn="✅ CRDT sync + vector clock"
                  mem="✅"
                  rag="❌"
                  hosted="✅"
                />
                <CompareRow
                  feature="Code-aware reads (AST outlines)"
                  cairn="✅ tree-sitter"
                  mem="❌"
                  rag="❌"
                  hosted="partial"
                />
                <CompareRow
                  feature="Edit verification (post-write)"
                  cairn="✅ built-in"
                  mem="❌"
                  rag="❌"
                  hosted="❌"
                />
                <CompareRow
                  feature="Drift detection on long tasks"
                  cairn="✅ sessions + checkpoints"
                  mem="❌"
                  rag="❌"
                  hosted="partial"
                />
                <CompareRow
                  feature="Self-hostable"
                  cairn="✅ single binary"
                  mem="partial (some open-source)"
                  rag="✅"
                  hosted="❌"
                />
                <CompareRow
                  feature="Pack sharing / federation"
                  cairn="✅ .cairnpkg + Ed25519"
                  mem="❌"
                  rag="❌"
                  hosted="❌"
                />
                <CompareRow
                  feature="E2E encryption for sync"
                  cairn="✅ Argon2id + ChaCha20"
                  mem="❌"
                  rag="partial"
                  hosted="✅"
                />
                <CompareRow
                  feature="No telemetry / cloud account"
                  cairn="✅"
                  mem="mixed"
                  rag="✅"
                  hosted="❌"
                />
                <CompareRow
                  feature="License"
                  cairn="Apache-2.0"
                  mem="mixed"
                  rag="mixed"
                  hosted="proprietary"
                />
              </tbody>
            </table>
          </div>
        </section>

        {/* Install paths */}
        <section className="mb-20">
          <h2 className="text-3xl font-semibold tracking-tight mb-3">
            Install in one command
          </h2>
          <p className="text-[#8A94A6] mb-6 max-w-2xl">
            Works on macOS, Linux, and Windows. The Docker stack is the easiest
            path — it brings up Cairn + HelixDB + MinIO with a single
            <code> docker compose up -d</code>.
          </p>
          <div className="grid sm:grid-cols-2 gap-4">
            <InstallCard label="Homebrew (macOS / Linux)" code="brew install cairn" />
            <InstallCard label="curl one-liner (Linux / macOS)" code="curl -fsSL https://cairn.sh/install.sh | sh" />
            <InstallCard label="PowerShell (Windows)" code="iwr cairn.sh/install.ps1 | iex" />
            <InstallCard label="Full stack (Docker)" code="cp .env.example .env && docker compose up -d" />
            <InstallCard label="From source" code="cargo install --git https://github.com/Vellixia/Cairn cairn-server cairn-cli" />
          </div>
        </section>

        {/* Trust signals */}
        <section className="mb-20">
          <h2 className="text-3xl font-semibold tracking-tight mb-3">
            Privacy + security, by default
          </h2>
          <div className="grid sm:grid-cols-2 gap-4">
            <Trust
              title="No telemetry, ever"
              body="Cairn does not phone home. The binary makes exactly two external calls in normal operation: the OS TLS cert chain, and whatever registry URL you point at."
            />
            <Trust
              title="Self-hosted, single binary"
              body="Rust, statically linked, no Node.js / Python / Docker required for the server. Runs on a Raspberry Pi."
            />
            <Trust
              title="Signed releases"
              body="Every release tarball is signed with keyless Sigstore cosign. The install script verifies SHA-256 + SLSA provenance before installing."
            />
            <Trust
              title="End-to-end encrypted sync"
              body="Sync envelopes can be encrypted with Argon2id-derived ChaCha20 keys. The server never sees plaintext when E2E is on."
            />
            <Trust
              title="Threat model documented"
              body="Every threat → mitigation pair lives in SECURITY.md. 10 rows in the v0.5.0 table; updates land in the same release."
            />
            <Trust
              title="Apache-2.0"
              body="Fork it, vendor it, ship it. No telemetry, no 'enterprise edition', no surprise license change."
            />
          </div>
        </section>

        {/* Footer */}
        <footer className="border-t border-[#222b35] pt-8 text-sm text-[#8A94A6]">
          <div className="flex flex-wrap gap-x-6 gap-y-2 mb-4">
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/docs/PLAN.md">Plan</a>
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/docs/ARCHITECTURE.md">Architecture</a>
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/docs/BENCHMARKS.md">Benchmarks</a>
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/docs/DECISIONS.md">ADRs</a>
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/docs/SECURITY.md">Security</a>
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/docs/ROADMAP.md">Roadmap</a>
            <a className="text-[#2DD4BF] hover:underline" href="https://github.com/Vellixia/Cairn/blob/main/CHANGELOG.md">Changelog</a>
          </div>
          <p>🪨 Cairn · Apache-2.0 · every traveler adds a stone.</p>
        </footer>
      </div>
    </main>
  );
}

function Row({
  label,
  before,
  after,
  pct,
}: {
  label: string;
  before: string;
  after: string;
  pct: string;
}) {
  return (
    <tr>
      <td className="py-3 px-4">{label}</td>
      <td className="py-3 px-4 text-right text-[#8A94A6] font-mono text-xs">{before}</td>
      <td className="py-3 px-4 text-right font-mono text-xs">{after}</td>
      <td className="py-3 px-4 text-right font-mono font-semibold text-[#2DD4BF]">{pct}</td>
    </tr>
  );
}

function CompareRow({
  feature,
  cairn,
  mem,
  rag,
  hosted,
}: {
  feature: string;
  cairn: string;
  mem: string;
  rag: string;
  hosted: string;
}) {
  return (
    <tr>
      <td className="py-3 px-4 font-medium">{feature}</td>
      <td className="py-3 px-4 text-[#2DD4BF]">{cairn}</td>
      <td className="py-3 px-4 text-[#8A94A6]">{mem}</td>
      <td className="py-3 px-4 text-[#8A94A6]">{rag}</td>
      <td className="py-3 px-4 text-[#8A94A6]">{hosted}</td>
    </tr>
  );
}

function InstallCard({ label, code }: { label: string; code: string }) {
  return (
    <div className="rounded-lg border border-[#222b35] bg-[#12181F] p-4">
      <div className="text-xs uppercase tracking-wider text-[#8A94A6] mb-2">
        {label}
      </div>
      <pre className="rounded bg-[#0B0F14] border border-[#222b35] p-3 text-xs font-mono text-[#ECEFF4] overflow-auto">
        <code>{code}</code>
      </pre>
    </div>
  );
}

function Trust({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-lg border border-[#222b35] bg-[#12181F] p-5">
      <h3 className="text-sm font-semibold text-[#FB923C] uppercase tracking-wider mb-2">
        {title}
      </h3>
      <p className="text-sm text-[#ECEFF4] leading-relaxed">{body}</p>
    </div>
  );
}

function DemoPlaceholder() {
  // v0.5.0 Phase 4.2 Sprint 17b: demo GIF placeholder. A real screen recording
  // is on the v0.5.1 roadmap (the cairn.sh landing page will embed it once the
  // recording pipeline is set up — see ADR-024).
  return (
    <section className="mb-20">
      <div className="rounded-lg border border-dashed border-[#FB923C] bg-[#12181F] overflow-hidden">
        <div className="aspect-video bg-gradient-to-br from-[#1a2129] via-[#12181F] to-[#0B0F14] flex items-center justify-center relative">
          <div className="text-center">
            <div className="text-[#FB923C] text-5xl font-bold tracking-tight mb-3">
              ▶ Demo
            </div>
            <p className="text-sm text-[#8A94A6] max-w-md mx-auto">
              A 30-second screen recording showing Cairn remembering a decision,
              recalling it across sessions, and verifying an edit — coming with
              v0.5.1.
            </p>
          </div>
          <div className="absolute top-3 left-3 text-[10px] uppercase tracking-wider text-[#8A94A6] bg-[#0B0F14] border border-[#222b35] rounded px-2 py-1">
            placeholder
          </div>
        </div>
      </div>
    </section>
  );
}
