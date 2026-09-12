"use client";

import { Lock } from "lucide-react";
import { ApiError } from "@/lib/api";
import { ErrorState } from "@/components/page";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";

/**
 * Snake_case vocabulary as prose.
 *
 * The server's status, stage, trigger and disposition vocabularies are closed
 * sets of identifiers, and every one of them is meant to be read by a person.
 * Nothing is translated or renamed here — an unknown value passes through with
 * its underscores turned into spaces, so a vocabulary this build has not heard
 * of still renders as itself rather than as a blank.
 */
export function humanize(value: string): string {
  return value.replace(/[_:]/g, " ");
}

/**
 * What the server said when it refused, shown as a refusal.
 *
 * **The UI is not the authority boundary — the API is.** Every gate in this
 * control plane is enforced server-side, so any page here can be reached by
 * someone the server will turn away: a non-member typing a project URL, a
 * member opening `/system`. Rendering nothing in that case would read as "there
 * is nothing here", which is exactly the confusion FR-894a exists to prevent.
 * A refusal is a result, and it is shown as one.
 */
export function ApiErrorState({ error }: { error: unknown }) {
  if (error instanceof ApiError && error.status === 403) {
    return (
      <Alert data-testid="refusal" role="alert">
        <Lock />
        <AlertTitle>You do not have access to this</AlertTitle>
        <AlertDescription>
          {/* The server's own words. It knows whether the gate was project
              membership or an administrator role; guessing here would sometimes
              be wrong and would always be a second opinion. */}
          {error.message}
        </AlertDescription>
      </Alert>
    );
  }
  return <ErrorState error={error} />;
}

/**
 * A labelled value in a detail panel.
 *
 * `value` is a node rather than a string so a caller can pass a badge, a
 * reference or a deliberate "not recorded" without this component deciding what
 * an absent value looks like — that decision belongs to whoever knows what
 * absence means for that field.
 */
export function Field({
  label,
  value,
  testId,
}: {
  label: string;
  value: React.ReactNode;
  testId?: string;
}) {
  return (
    <div className="min-w-0">
      <dt className="text-muted-foreground text-xs">{label}</dt>
      <dd className="mt-0.5 text-sm break-words" data-testid={testId}>
        {value}
      </dd>
    </div>
  );
}

/** A field the server did not record, said in words rather than left blank. */
export function NotRecorded({ what = "not recorded" }: { what?: string }) {
  return <span className="text-muted-foreground italic">{what}</span>;
}

/**
 * The bound a list was read under, stated whenever it may have hidden rows.
 *
 * A page that came back exactly full is indistinguishable from a collection
 * that happens to be exactly that size, so the notice appears in both cases —
 * saying "there may be more" when there is not is a smaller error than
 * silently truncating (FR-895).
 */
export function TruncationNotice({
  shown,
  limit,
  refine,
}: {
  shown: number;
  limit: number;
  refine: string;
}) {
  if (shown < limit) return null;
  return (
    <p className="text-muted-foreground mt-3 text-xs" data-testid="truncated">
      Showing the first {limit}. {refine}
    </p>
  );
}
