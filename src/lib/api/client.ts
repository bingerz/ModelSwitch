// HTTP transport layer.
// Exports the default `request()` (admin_token via localStorage) plus a
// `createRequestFn()` factory so other surfaces (e.g. Portal) can plug in
// their own token source and 401 handler. Also hosts the Tauri bridge.

import { API_BASE, isTauri } from "../runtime";

export { isTauri };

/**
 * Build the Authorization header pair for a given bearer token.
 * Returns an empty object when the token is missing so callers can spread
 * the result without conditionals. Centralised here to kill the prior
 * triple-duplication of `token ? { Authorization: `Bearer ${token}` } : {}`.
 */
export function buildAuthHeaders(token: string | null): Record<string, string> {
  return token ? { Authorization: `Bearer ${token}` } : {};
}

/** Default 401 handler for the admin surface: clear token, flag, reload. */
function defaultUnauthorizedHandler(): void {
  localStorage.removeItem("admin_token");
  sessionStorage.setItem("auth_expired", "1");
  window.location.reload();
}

export interface RequestFnConfig {
  /**
   * Invoked on HTTP 401 before the "Unauthorized" error is thrown.
   * Defaults to the admin clear-and-reload flow. Portal supplies its own
   * to drop the portal_token without forcing a full page reload.
   */
  onUnauthorized?: () => void;
}

/**
 * Factory that produces a `request<T>()` function bound to a specific
 * token source. Shares envelope-unwrap + 401 + error-handling logic across
 * surfaces that authenticate with different storage keys.
 */
export function createRequestFn(
  getToken: () => string | null,
  config?: RequestFnConfig,
) {
  return async function request<T>(path: string, init?: RequestInit): Promise<T> {
    const token = getToken();
    const res = await fetch(`${API_BASE}${path}`, {
      ...init,
      headers: {
        "Content-Type": "application/json",
        ...buildAuthHeaders(token),
        ...init?.headers,
      },
    });
    if (res.status === 401) {
      if (config?.onUnauthorized) {
        config.onUnauthorized();
      } else {
        defaultUnauthorizedHandler();
      }
      throw new Error("Unauthorized");
    }
    if (!res.ok) throw new Error(`API error: ${res.status}`);
    const json = await res.json();
    // Auto-unwrap ApiResponse<T> envelope
    if (json && typeof json === "object" && "ok" in json && "data" in json) {
      if (!json.ok) {
        throw new Error(json.error?.message ?? "Unknown API error");
      }
      return json.data as T;
    }
    return json as T;
  };
}

/**
 * Default transport for the admin surface — reads the bearer token from
 * `localStorage.getItem("admin_token")` and uses the admin clear-and-reload
 * 401 handler.
 */
export const request = createRequestFn(() => localStorage.getItem("admin_token"));

/**
 * Raw text fetcher factory (used for Prometheus `/metrics`) — same auth +
 * error-handling semantics as `request`, but returns the response body as
 * text rather than parsing JSON.
 */
export function createRawTextFn(
  getToken: () => string | null,
  config?: RequestFnConfig,
) {
  return async function fetchText(path: string): Promise<string> {
    const token = getToken();
    const res = await fetch(`${API_BASE}${path}`, {
      headers: buildAuthHeaders(token),
    });
    if (res.status === 401) {
      if (config?.onUnauthorized) {
        config.onUnauthorized();
      } else {
        defaultUnauthorizedHandler();
      }
      throw new Error("Unauthorized");
    }
    if (!res.ok) throw new Error(`API error: ${res.status}`);
    return res.text();
  };
}

/** Default raw-text fetcher for the admin surface (Prometheus metrics). */
export const fetchText = createRawTextFn(() => localStorage.getItem("admin_token"));

/** Tauri bridge — proxies desktop IPC commands when running inside Tauri. */
export async function invokeTauri<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) {
    throw new Error(`Command "${cmd}" is only available in desktop mode`);
  }
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  return tauriInvoke(cmd, args);
}
