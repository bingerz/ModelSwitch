import { vi } from "vitest";

/**
 * Factory for the react-i18next mock.
 * Returns translation keys as-is, interpolating `{param}` placeholders.
 *
 * Usage in test files — `vi.mock` must be called at the top level so
 * Vitest can hoist it above imports:
 *
 *   import { i18nMockFactory } from "../test/i18n-mock";
 *   vi.mock("react-i18next", i18nMockFactory);
 */
export const i18nMockFactory = () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) => {
      if (params) {
        return Object.entries(params).reduce(
          (str, [k, v]) => str.replace(`{${k}}`, String(v)),
          key,
        );
      }
      return key;
    },
    i18n: { language: "en", changeLanguage: vi.fn() },
  }),
});

/**
 * Convenience wrapper — call at the top of a test file before component imports.
 * Note: due to Vitest's ESM hoisting, prefer `vi.mock("react-i18next", i18nMockFactory)`
 * directly in each test file for reliable hoisting.
 */
export function mockI18n() {
  vi.mock("react-i18next", i18nMockFactory);
}
