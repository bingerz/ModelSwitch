import { useEffect, type RefObject } from "react";

/**
 * Focus trap hook for modal dialogs.
 *
 * When `isActive` is true:
 *  - Moves focus to the first focusable element inside the container
 *  - Wraps Tab / Shift+Tab so focus stays inside the container
 *  - Calls `onEscape` when the Escape key is pressed (if provided)
 *
 * Cleanup runs on unmount or when `isActive` becomes false.
 */
export function useFocusTrap(
  ref: RefObject<HTMLElement | null>,
  isActive: boolean,
  onEscape?: () => void,
): void {
  useEffect(() => {
    if (!isActive || !ref.current) return;
    const element = ref.current;

    const getFocusable = (): HTMLElement[] =>
      Array.from(
        element.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((el) => !el.hasAttribute("disabled"));

    const focusable = getFocusable();
    if (focusable.length === 0) return;

    const first = focusable[0];

    first.focus();

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (onEscape) {
          e.preventDefault();
          onEscape();
        }
        return;
      }
      if (e.key !== "Tab") return;
      const currentFocusable = getFocusable();
      if (currentFocusable.length === 0) return;
      const currentFirst = currentFocusable[0];
      const currentLast = currentFocusable[currentFocusable.length - 1];
      if (e.shiftKey) {
        if (document.activeElement === currentFirst) {
          e.preventDefault();
          currentLast.focus();
        }
      } else {
        if (document.activeElement === currentLast) {
          e.preventDefault();
          currentFirst.focus();
        }
      }
    };

    element.addEventListener("keydown", handleKeyDown);
    return () => element.removeEventListener("keydown", handleKeyDown);
  }, [ref, isActive, onEscape]);
}
