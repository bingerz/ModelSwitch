import { describe, it, expect } from "vitest";
import {
  PROVIDER_PRESETS,
  groupPresetsByCategory,
  type PresetCategory,
  type ApiFormat,
} from "./presets";

const VALID_CATEGORIES: PresetCategory[] = [
  "official",
  "cn_official",
  "aggregator",
  "cloud_provider",
  "third_party",
];

const VALID_PROVIDERS = [
  "openai",
  "anthropic",
  "deepseek",
  "gemini",
  "openrouter",
  "custom",
];

const VALID_API_FORMATS: ApiFormat[] = ["openai", "anthropic", "gemini"];

const VALID_QUOTA_STRATEGIES = [
  "http_api",
  "openai_compat",
  "webview",
  "response_header",
  "disabled",
];

describe("PROVIDER_PRESETS data integrity", () => {
  it("every preset has a non-empty name", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.name, `Preset with empty name`).toBeTruthy();
      expect(p.name.length).toBeGreaterThan(0);
    }
  });

  it("every preset has a valid category", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.category, `${p.name} has invalid category`).toBeOneOf(VALID_CATEGORIES);
    }
  });

  it("every preset has a valid provider", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.provider, `${p.name} has invalid provider "${p.provider}"`).toBeOneOf(
        VALID_PROVIDERS,
      );
    }
  });

  it("every preset has a non-empty baseUrl", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.baseUrl, `${p.name} missing baseUrl`).toBeTruthy();
      expect(p.baseUrl).toMatch(/^https?:\/\//);
    }
  });

  it("every preset has a non-empty models array", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.models.length, `${p.name} has empty models`).toBeGreaterThan(0);
    }
  });

  it("defaultModel exists in models array", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(
        p.models.includes(p.defaultModel),
        `${p.name}: defaultModel "${p.defaultModel}" not in models [${p.models.join(", ")}]`,
      ).toBe(true);
    }
  });

  it("every preset has priority between 1 and 3", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.priority, `${p.name} has priority ${p.priority}`).toBeGreaterThanOrEqual(1);
      expect(p.priority).toBeLessThanOrEqual(3);
    }
  });

  it("every preset has a valid hex iconColor", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.iconColor, `${p.name} missing iconColor`).toMatch(/^#[0-9A-Fa-f]{6}$/);
    }
  });

  it("no duplicate preset names", () => {
    const names = PROVIDER_PRESETS.map((p) => p.name);
    const unique = new Set(names);
    expect(unique.size, `Duplicate names found`).toBe(names.length);
  });

  it("no duplicate baseUrls (excluding Azure placeholder)", () => {
    const urls = PROVIDER_PRESETS.filter((p) => !p.baseUrl.includes("YOUR_RESOURCE")).map(
      (p) => p.baseUrl,
    );
    const unique = new Set(urls);
    expect(unique.size, `Duplicate baseUrl found`).toBe(urls.length);
  });

  it("modelMapping values are non-empty strings", () => {
    for (const p of PROVIDER_PRESETS) {
      for (const [key, val] of Object.entries(p.modelMapping)) {
        expect(key, `${p.name} modelMapping key empty`).toBeTruthy();
        expect(val, `${p.name} modelMapping["${key}"] value empty`).toBeTruthy();
      }
    }
  });

  it("apiFormat is valid when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.apiFormat !== undefined) {
        expect(p.apiFormat, `${p.name} has invalid apiFormat`).toBeOneOf(VALID_API_FORMATS);
      }
    }
  });

  it("endpoints use valid API format keys when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.endpoints) {
        for (const key of Object.keys(p.endpoints)) {
          expect(VALID_API_FORMATS.includes(key as ApiFormat), `${p.name} has invalid endpoint key "${key}"`).toBe(true);
        }
      }
    }
  });

  it("quotaStrategy is valid when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.quotaStrategy !== undefined) {
        expect(p.quotaStrategy, `${p.name} has invalid quotaStrategy`).toBeOneOf(
          VALID_QUOTA_STRATEGIES,
        );
      }
    }
  });

  it("apiKeyUrl is a valid URL when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.apiKeyUrl !== undefined) {
        expect(p.apiKeyUrl, `${p.name} apiKeyUrl invalid`).toMatch(/^https?:\/\//);
      }
    }
  });
});

describe("groupPresetsByCategory", () => {
  it("returns all 5 categories as keys", () => {
    const groups = groupPresetsByCategory();
    expect(Object.keys(groups).sort()).toEqual(
      ["official", "cn_official", "aggregator", "cloud_provider", "third_party"].sort(),
    );
  });

  it("total presets across all groups equals PROVIDER_PRESETS length", () => {
    const groups = groupPresetsByCategory();
    const total = Object.values(groups).reduce((sum, arr) => sum + arr.length, 0);
    expect(total).toBe(PROVIDER_PRESETS.length);
  });

  it("every preset appears in exactly one group", () => {
    const groups = groupPresetsByCategory();
    for (const preset of PROVIDER_PRESETS) {
      const count = Object.values(groups).filter((arr) => arr.includes(preset)).length;
      expect(count, `${preset.name} appears in ${count} groups`).toBe(1);
    }
  });

  it("official category contains known providers", () => {
    const groups = groupPresetsByCategory();
    const officialNames = groups.official.map((p) => p.name);
    expect(officialNames).toContain("OpenAI");
    expect(officialNames).toContain("Anthropic");
  });

  it("cn_official category is non-empty", () => {
    const groups = groupPresetsByCategory();
    expect(groups.cn_official.length).toBeGreaterThan(0);
  });
});
