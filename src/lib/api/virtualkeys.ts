// Virtual key endpoints, exposed as the nested `api.virtualKeys` object.

import { request } from "./client";
import type {
  BatchCreateVirtualKeyData,
  BatchCreateVirtualKeyItem,
  CreateVirtualKeyData,
  CreateVirtualKeyResponse,
  ListVirtualKeysParams,
  PaginatedVirtualKeys,
  UpdateVirtualKeyData,
  VirtualKey,
} from "./types";

export const virtualKeysApi = {
  list: (params?: ListVirtualKeysParams) => {
    const search = new URLSearchParams();
    if (params?.page) search.set("page", String(params.page));
    if (params?.limit) search.set("limit", String(params.limit));
    if (params?.search) search.set("search", params.search);
    if (params?.group) search.set("group", params.group);
    const qs = search.toString();
    return request<PaginatedVirtualKeys>(
      qs ? `/api/virtual-keys?${qs}` : "/api/virtual-keys",
    );
  },

  create: (data: CreateVirtualKeyData) =>
    request<CreateVirtualKeyResponse>("/api/virtual-keys", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  batchCreate: (data: BatchCreateVirtualKeyData) =>
    request<BatchCreateVirtualKeyItem[]>("/api/virtual-keys/batch", {
      method: "POST",
      body: JSON.stringify(data),
    }),

  update: (id: string, data: UpdateVirtualKeyData) =>
    request<VirtualKey>(`/api/virtual-keys/${id}`, {
      method: "PUT",
      body: JSON.stringify(data),
    }),

  delete: (id: string) =>
    request<void>(`/api/virtual-keys/${id}`, { method: "DELETE" }),

  groups: () => request<string[]>("/api/virtual-keys/groups"),
};
