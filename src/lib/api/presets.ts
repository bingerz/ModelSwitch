// UI presets and helpers that previously lived at the bottom of api.ts.
// Separated so the transport/domain modules stay focused.

export const PRIORITY_TIERS: Record<number, { color: string }> = {
  1: { color: "var(--color-success)" },
  2: { color: "var(--color-warning)" },
  3: { color: "var(--color-danger)" },
};

export function validateChannelForm(fields: {
  name: string;
  baseUrl: string;
}): string | null {
  if (!fields.name.trim()) return "channels.nameRequired";
  if (!fields.baseUrl.trim()) return "channels.baseUrlRequired";
  if (
    !fields.baseUrl.startsWith("http://") &&
    !fields.baseUrl.startsWith("https://")
  ) {
    return "channels.baseUrlInvalid";
  }
  return null;
}
