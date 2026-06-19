"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { getJSON, type Health, type Me } from "@/lib/api";

interface Props {
  me: Me;
}

export function Topbar({ me }: Props) {
  const router = useRouter();
  const [health, setHealth] = useState<Health | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const h = await getJSON<Health>("/api/health");
        if (!cancelled) setHealth(h);
      } catch {
        if (!cancelled) setHealth(null);
      }
    };
    poll();
    const id = window.setInterval(poll, 15_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  async function logout() {
    try {
      await fetch("/api/auth/logout", {
        method: "POST",
        credentials: "include",
      });
    } catch {
      // ignore — cookie clear is best-effort
    }
    router.replace("/login");
  }

  const status = health ? (
    <span className="flex items-center gap-1.5 text-xs">
      <span className="h-2 w-2 rounded-full bg-teal" aria-hidden />
      <span className="text-slate">healthy</span>
    </span>
  ) : (
    <span className="flex items-center gap-1.5 text-xs">
      <span className="h-2 w-2 rounded-full bg-[#f87171]" aria-hidden />
      <span className="text-slate">offline</span>
    </span>
  );

  return (
    <header className="sticky top-0 z-10 border-b border-line bg-ink/80 backdrop-blur">
      <div className="flex items-center justify-between gap-4 px-5 py-3">
        <div className="flex items-center gap-3 text-sm">
          <kbd className="hidden sm:inline-flex items-center gap-1 rounded border border-line bg-surface2 px-1.5 py-0.5 font-mono text-[11px] text-slate">
            ⌘K
          </kbd>
          <span className="text-slate hidden sm:inline">jump to anything</span>
        </div>
        <div className="flex items-center gap-4">
          {status}
          <span className="text-xs text-slate hidden sm:inline">
            signed in as <span className="text-offwhite font-medium">{me.username}</span>
          </span>
          <div className="relative">
            <button
              type="button"
              onClick={() => setMenuOpen((v) => !v)}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              className="rounded-full border border-line bg-surface2 px-2.5 py-1 text-xs hover:bg-surface"
            >
              {me.username.slice(0, 1).toUpperCase()}
            </button>
            {menuOpen && (
              <div
                role="menu"
                className="absolute right-0 mt-2 w-44 rounded-lg border border-line bg-surface2 py-1 shadow-lg shadow-black/30"
              >
                <a
                  href="/dashboard/settings"
                  role="menuitem"
                  className="block px-3 py-1.5 text-sm hover:bg-surface"
                  onClick={() => setMenuOpen(false)}
                >
                  Settings
                </a>
                <button
                  type="button"
                  role="menuitem"
                  onClick={logout}
                  className="block w-full px-3 py-1.5 text-left text-sm text-[#f87171] hover:bg-surface"
                >
                  Sign out
                </button>
              </div>
            )}
          </div>
        </div>
      </div>
    </header>
  );
}
