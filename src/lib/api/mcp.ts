// MCP server management endpoints, exposed as the nested `api.mcp` object
// plus the flat `api.mcpHealth` method.

import { request } from "./client";
import type {
  CreateMcpServerData,
  McpHealthEntry,
  McpServer,
  McpToolDetail,
  McpToolInfo,
  UpdateMcpServerData,
} from "./types";

export const mcpApi = {
  listServers: () => request<McpServer[]>("/api/mcp/servers"),

  createServer: (data: CreateMcpServerData) =>
    request<McpServer>("/api/mcp/servers", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  updateServer: (id: string, data: UpdateMcpServerData) =>
    request<McpServer>(`/api/mcp/servers/${id}`, {
      method: "PUT",
      body: JSON.stringify(data),
    }),

  deleteServer: (id: string) =>
    request<void>(`/api/mcp/servers/${id}`, { method: "DELETE" }),

  startServer: (id: string) =>
    request<{ ok: boolean }>(`/api/mcp/servers/${id}/start`, { method: "POST" }),

  stopServer: (id: string) =>
    request<{ ok: boolean }>(`/api/mcp/servers/${id}/stop`, { method: "POST" }),

  listServerTools: (id: string) =>
    request<McpToolDetail[]>(`/api/mcp/servers/${id}/tools`),

  listAllTools: () => request<McpToolInfo[]>("/api/mcp/tools"),
};

/** Flat method also exposed at `api.mcpHealth` for backward compat. */
export const mcpHealth = () => request<McpHealthEntry[]>("/api/mcp/health");
