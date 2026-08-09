"use client";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";

/**
 * A destructive action behind one question.
 *
 * Deleting a memory and revoking a token are both irreversible and both used to
 * happen on a single stray click, next to rows a reader is scanning.
 */
export function ConfirmButton({
  title,
  description,
  confirmLabel,
  onConfirm,
  disabled,
  ariaLabel,
  testId,
  children,
}: {
  title: string;
  description: React.ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  disabled?: boolean;
  ariaLabel: string;
  testId?: string;
  children: React.ReactNode;
}) {
  return (
    <AlertDialog>
      <AlertDialogTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            aria-label={ariaLabel}
            data-testid={testId}
            disabled={disabled}
          />
        }
      >
        {children}
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel render={<Button variant="outline" />}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction
            render={<Button variant="destructive" />}
            onClick={onConfirm}
            data-testid="confirm"
          >
            {confirmLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
