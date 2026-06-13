import Link from "next/link";
import Logo from "@/components/Logo";

const PILLARS = [
  { title: "Remember", body: "Decisions, tasks, and rationale persist across sessions, devices, and agents. Never start cold." },
  { title: "Compress — no loss", body: "Files, shell output, and responses shrink in the window but stay byte-identical recoverable on demand." },
  { title: "Assemble lean context", body: "Fight context rot: feed less, higher-signal, well-ordered context under a token budget." },
  { title: "Stay reliable", body: "Verify agent edits against the retained originals, detect drift, and re-anchor long tasks." },
  { title: "Smarter together", body: "Learn your preferences and opt into sanitized, federated collective knowledge." },
];

const PROBLEMS = [
  ["Forgets everything", "Every new session starts from zero — decisions and rationale are gone."],
  ["Re-reads the same files", "A 1000-line file gets read in full again just to rebuild context."],
  ["Decays on long tasks", "Context rot and reasoning drift compound silently over hours."],
  ["Siloed per device & tool", "Memory is trapped on one machine and inside one agent."],
];

function Cmd({ children }: { children: React.ReactNode }) {
  return (
    <code className="block rounded-lg border border-line bg-surface2 px-4 py-3 font-mono text-sm text-offwhite">
      {children}
    </code>
  );
}

export default function Home() {
  return (
    <main>
      {/* nav */}
      <header className="mx-auto flex max-w-5xl items-center justify-between px-5 py-5">
        <div className="flex items-center gap-2.5">
          <Logo size={30} />
          <span className="text-lg font-semibold tracking-tight">Cairn</span>
        </div>
        <nav className="flex items-center gap-5 text-sm text-slate">
          <a href="https://github.com/cairn-dev/cairn" className="hover:text-offwhite">GitHub</a>
          <Link href="/dashboard" className="rounded-lg bg-ember px-3.5 py-1.5 font-semibold text-[#1a1206] hover:opacity-90">
            Open dashboard
          </Link>
        </nav>
      </header>

      {/* hero */}
      <section className="mx-auto max-w-5xl px-5 pb-10 pt-12 text-center">
        <p className="mb-3 text-sm font-medium uppercase tracking-[0.18em] text-teal">
          The open-source context &amp; reliability layer for AI agents
        </p>
        <h1 className="mx-auto max-w-3xl text-5xl font-bold leading-[1.05] tracking-tight">
          Make any model <span className="text-ember">smart</span>.
        </h1>
        <p className="mx-auto mt-5 max-w-2xl text-lg text-[#cdd5e0]">
          Cairn remembers everything, feeds lean context, and keeps your agents reliable on long
          tasks — self-hosted, one Rust binary, with <span className="text-offwhite">no context ever lost</span>.
        </p>
        <div className="mx-auto mt-8 flex max-w-xl flex-col gap-3">
          <Cmd>curl -fsSL https://cairn.sh/install.sh | sh</Cmd>
          <div className="flex justify-center gap-3 text-sm">
            <Link href="/dashboard" className="rounded-lg bg-ember px-4 py-2 font-semibold text-[#1a1206] hover:opacity-90">
              Open the dashboard →
            </Link>
            <a href="https://github.com/cairn-dev/cairn" className="rounded-lg border border-line px-4 py-2 font-semibold hover:bg-surface">
              Star on GitHub
            </a>
          </div>
        </div>
      </section>

      {/* problem strip */}
      <section className="mx-auto max-w-5xl px-5 py-10">
        <h2 className="mb-1 text-center text-sm uppercase tracking-[0.14em] text-slate">
          Why long agent sessions fall apart
        </h2>
        <p className="mx-auto mb-7 max-w-2xl text-center text-[#aab3c0]">
          The bottleneck usually isn&apos;t the model&apos;s IQ — it&apos;s the context fed to it and the drift over time.
        </p>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {PROBLEMS.map(([t, b]) => (
            <div key={t} className="rounded-xl border border-line bg-surface p-4">
              <div className="mb-1 font-semibold text-offwhite">{t}</div>
              <div className="text-sm text-slate">{b}</div>
            </div>
          ))}
        </div>
      </section>

      {/* pillars */}
      <section className="mx-auto max-w-5xl px-5 py-10">
        <h2 className="mb-7 text-center text-2xl font-semibold tracking-tight">Five pillars, one engine</h2>
        <div className="grid gap-4 md:grid-cols-2">
          {PILLARS.map((p, i) => (
            <div key={p.title} className="rounded-xl border border-line bg-surface p-5">
              <div className="mb-2 flex items-center gap-3">
                <span className="flex h-7 w-7 items-center justify-center rounded-full bg-surface2 font-mono text-xs text-ember">
                  {i + 1}
                </span>
                <h3 className="font-semibold">{p.title}</h3>
              </div>
              <p className="text-sm text-slate">{p.body}</p>
            </div>
          ))}
        </div>
      </section>

      {/* install / self-host */}
      <section className="mx-auto max-w-5xl px-5 py-10">
        <div className="rounded-2xl border border-line bg-surface p-7">
          <h2 className="text-2xl font-semibold tracking-tight">Self-host in one command</h2>
          <p className="mt-2 max-w-2xl text-[#aab3c0]">
            One tiny Rust binary — no Node or Python runtime. Run it on a home server, NAS,
            Raspberry Pi, or a cheap VPS, then pair every device from the dashboard.
          </p>
          <div className="mt-5 grid gap-3 md:grid-cols-2">
            <div>
              <div className="mb-2 text-xs uppercase tracking-wider text-slate">Server</div>
              <Cmd>docker compose up</Cmd>
            </div>
            <div>
              <div className="mb-2 text-xs uppercase tracking-wider text-slate">Each device (from the dashboard)</div>
              <Cmd>curl -fsSL https://cairn.sh/i | sh -s -- pair CAIRN-7Q3X</Cmd>
            </div>
          </div>
          <p className="mt-4 text-sm text-slate">
            Pairing auto-detects your agents and wires up Claude Code, Codex, Cursor, OpenCode,
            Windsurf, and more.
          </p>
        </div>
      </section>

      <footer className="mx-auto max-w-5xl px-5 py-10 text-sm text-slate">
        <div className="flex items-center gap-2">
          <Logo size={18} />
          <span>Cairn — Apache-2.0 · every traveler adds a stone.</span>
        </div>
      </footer>
    </main>
  );
}
