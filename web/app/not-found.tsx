import Link from "next/link";
import { Compass } from "lucide-react";
import { Button } from "@/components/ui/button";

export default function NotFound() {
  return (
    <div className="flex min-h-svh flex-col items-center justify-center gap-4 p-6 text-center">
      <div className="bg-muted text-muted-foreground flex size-12 items-center justify-center rounded-lg">
        <Compass className="size-6" strokeWidth={1.5} />
      </div>
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Page not found</h1>
        <p className="text-muted-foreground mt-1 text-sm text-balance">
          That address does not match anything in Cairn.
        </p>
      </div>
      <Button render={<Link href="/" />}>Back to projects</Button>
    </div>
  );
}
