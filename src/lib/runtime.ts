/** Detect whether we're running in Tauri desktop or web browser mode. */
const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

/** When served by the gateway (web mode), API calls go to same origin (empty string).
 *  In Tauri mode, they go to the local gateway HTTP server. */
const API_BASE = isTauri ? "http://127.0.0.1:8080" : "";

export { isTauri, API_BASE };
