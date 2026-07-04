import { useEffect } from "react";
import { useToast } from "../components/Toast";

/**
 * Shows a toast notification when a TanStack Query encounters an error.
 * Only fires when `isError` flips to true — not on every re-render while
 * the error state persists — so the user sees one notification per failure.
 */
export function useQueryToastError(
  isError: boolean,
  message: string = "Failed to load data",
): void {
  const toast = useToast();
  useEffect(() => {
    if (isError) {
      toast.error(message);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isError]);
}
