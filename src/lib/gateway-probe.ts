import { API_BASE } from "./runtime";

/** Quick health probe — true when the gateway HTTP server responds. */
export async function probeGatewayHealth(
  timeoutMs = 2000,
): Promise<boolean> {
  try {
    const res = await fetch(`${API_BASE}/health`, {
      signal: AbortSignal.timeout(timeoutMs),
    });
    return res.ok;
  } catch {
    return false;
  }
}
