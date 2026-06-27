// Lightweight mock-mode flag — synchronous, tiny, ships in the main bundle.
//
// The heavy mock data + mock API implementation live in ./mock.ts and
// ./mock-data.ts and are loaded dynamically only when mock mode is on.

/**
 * Returns true when mock mode is active. Reads from the URL query string
 * (`?mock=1`) or localStorage (`msw-mock=1`) so it can be toggled without
 * code changes.
 */
export function isMockMode(): boolean {
  try {
    const url = new URL(window.location.href);
    const mockParam = url.searchParams.get("mock");
    if (mockParam === "true" || mockParam === "1" || mockParam === "") return true;
  } catch {
    // SSR or non-browser env
  }
  try {
    return localStorage.getItem("msw-mock") === "1";
  } catch {
    return false;
  }
}

/**
 * Enable/disable mock mode. Persists to localStorage and reloads the page so
 * the dynamic mock module is re-evaluated with the new flag value.
 */
export function setMockMode(enabled: boolean): void {
  try {
    if (enabled) {
      localStorage.setItem("msw-mock", "1");
    } else {
      localStorage.removeItem("msw-mock");
    }
  } catch {
    // localStorage not available
  }
  window.location.reload();
}
