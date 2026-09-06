// Model fetching API: fetch available models from provider endpoints

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

export const modelsApi = {
  fetchProviderModels,
};
