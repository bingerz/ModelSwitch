// Shared types, constants, and pure helpers for the MCP servers panel.
// Kept in a `.ts` file so siblings can import without pulling JSX.

import type {
  McpServer,
  McpServerStatus,
  McpToolDetail,
  CreateMcpServerData,
  UpdateMcpServerData,
} from "../../lib/api";

// Re-export shared API types so sibling modules can import from a single location.
export type {
  McpServer,
  McpServerStatus,
  McpToolDetail,
  CreateMcpServerData,
  UpdateMcpServerData,
};

// ─── Status Helpers ─────────────────────────────────────

export function statusIsRunning(status: McpServerStatus): boolean {
  return typeof status === "object" && status !== null && "running" in status;
}

export function statusIsError(status: McpServerStatus): boolean {
  return typeof status === "object" && status !== null && "error" in status;
}

export function statusToolCount(status: McpServerStatus): number {
  if (statusIsRunning(status)) {
    return (status as { running: { tool_count: number } }).running.tool_count;
  }
  return 0;
}

export function statusErrorMessage(status: McpServerStatus): string | null {
  if (statusIsError(status)) {
    return (status as { error: { message: string } }).error.message;
  }
  return null;
}

export const STATUS_COLOR: Record<string, string> = {
  stopped: "var(--color-text-muted)",
  running: "var(--color-success)",
  error: "var(--color-danger)",
};

export function statusKey(status: McpServerStatus): string {
  if (status === "stopped") return "stopped";
  if (statusIsRunning(status)) return "running";
  if (statusIsError(status)) return "error";
  return "stopped";
}

// ─── Text Helpers ───────────────────────────────────────

export function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max) + "..." : s;
}

// ─── Env Helpers ────────────────────────────────────────

export function parseEnvText(text: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eqIdx = trimmed.indexOf("=");
    if (eqIdx === -1) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    const value = trimmed.slice(eqIdx + 1).trim();
    if (key) env[key] = value;
  }
  return env;
}

export function envToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

export function parseArgsText(text: string): string[] {
  return text
    .trim()
    .split(/\s+/)
    .filter((s) => s.length > 0);
}

export function argsToText(args: string[]): string {
  return args.join(" ");
}
