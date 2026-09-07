// Model fetching and endpoint probing API

import { invokeTauri } from "./client";

export interface FetchedModel {
  id: string;
  ownedBy?: string;
}

export interface FetchModelsParams {
  baseUrl: string;
  apiKey: string;
  isFullUrl?: boolean;
  modelsUrl?: string;
  apiFormat?: string;
}

export interface ProbeResult {
  url: string;
  ok: boolean;
  status?: number;
  latencyMs?: number;
  error?: string;
  reachable: boolean;
}

/**
 * Fetch available models from a provider using OpenAI-compatible /v1/models endpoint.
 * Tries multiple candidate URLs generated from baseUrl, with smart handling of
 * Anthropic-compatible subpaths (e.g. /anthropic, /apps/anthropic, /coding, etc.).
 */
export async function fetchProviderModels(params: FetchModelsParams): Promise<FetchedModel[]> {
  return invokeTauri<FetchedModel[]>("fetch_provider_models", {
    baseUrl: params.baseUrl,
    apiKey: params.apiKey,
    isFullUrl: params.isFullUrl ?? false,
    modelsUrl: params.modelsUrl,
    apiFormat: params.apiFormat,
  });
}

/**
 * Probe multiple endpoints to measure latency and reachability.
 * Used for endpointCandidates speed-test UI.
 */
export async function probeEndpoints(urls: string[]): Promise<ProbeResult[]> {
  return invokeTauri<ProbeResult[]>("probe_endpoints", { urls });
}

export const modelsApi = {
  fetchProviderModels,
  probeEndpoints,
};
