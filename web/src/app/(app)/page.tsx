import { OverviewContent } from "./OverviewContent";

export default function HomePage() {
  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">Now</h1>
        <p className="text-sm text-muted-foreground">
          Server health, reliability, recent memory, and the last few admin actions — at a glance.
        </p>
      </header>
      <OverviewContent />
    </div>
  );
}