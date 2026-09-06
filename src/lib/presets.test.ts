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

  it("no unexpected duplicate baseUrls (product-line splits allowed)", () => {
    // Allow duplicates for product-line variants that share endpoints but differ in auth/tier
    const allowedDuplicates = [
      "https://ark.cn-beijing.volces.com/api/coding", // DouBao / 火山 Coding Plan
      "https://bedrock-runtime.us-west-2.amazonaws.com", // AWS Bedrock (AKSK) / (API Key)
      "https://tokenhub-intl.tencentcloudmaas.com/plan/anthropic", // Tencent variants
      "https://tokenhub.tencentmaas.com/plan/anthropic", // Tencent variants
    ];
    const urls = PROVIDER_PRESETS.filter((p) => !p.baseUrl.includes("YOUR_RESOURCE")).map(
      (p) => p.baseUrl,
    );
    const urlCounts = new Map<string, number>();
    for (const url of urls) {
      urlCounts.set(url, (urlCounts.get(url) || 0) + 1);
    }
    for (const [url, count] of urlCounts.entries()) {
      if (count > 1 && !allowedDuplicates.includes(url)) {
        expect.fail(`Unexpected duplicate baseUrl: ${url} (${count} times)`);
      }
    }
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

  it("websiteUrl is a valid URL when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.websiteUrl !== undefined) {
        expect(p.websiteUrl, `${p.name} websiteUrl invalid`).toMatch(/^https?:\/\//);
      }
    }
  });

  it("modelsUrl is a valid URL when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.modelsUrl !== undefined) {
        expect(p.modelsUrl, `${p.name} modelsUrl invalid`).toMatch(/^https?:\/\//);
      }
    }
  });

  it("endpointCandidates are valid URLs when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.endpointCandidates !== undefined) {
        expect(p.endpointCandidates.length, `${p.name} endpointCandidates empty`).toBeGreaterThan(0);
        for (const url of p.endpointCandidates) {
          expect(url, `${p.name} endpointCandidate "${url}" invalid`).toMatch(/^https?:\/\//);
        }
      }
    }
  });

  it("templateValues have required structure when present", () => {
    for (const p of PROVIDER_PRESETS) {
      if (p.templateValues !== undefined) {
        expect(typeof p.templateValues, `${p.name} templateValues must be object`).toBe("object");
        for (const [key, config] of Object.entries(p.templateValues)) {
          expect(config.label, `${p.name} templateValues["${key}"].label missing`).toBeTruthy();
          expect(config.placeholder, `${p.name} templateValues["${key}"].placeholder missing`).toBeTruthy();
          expect(typeof config.editorValue, `${p.name} templateValues["${key}"].editorValue must be string`).toBe("string");
        }
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

describe("New P0 presets added", () => {
  it("includes Kimi For Coding preset", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "Kimi For Coding");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("coding");
    expect(preset?.icon).toBe("kimi");
  });

  it("includes Bailian For Coding preset", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "Bailian For Coding");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("coding");
    expect(preset?.icon).toBe("bailian");
  });

  it("includes QwenCloud variants", () => {
    const qwencloud = PROVIDER_PRESETS.find((p) => p.name === "QwenCloud");
    const qwencloudCoding = PROVIDER_PRESETS.find((p) => p.name === "QwenCloud For Coding");
    const qwencloudToken = PROVIDER_PRESETS.find((p) => p.name === "QwenCloud Token Plan");
    expect(qwencloud).toBeTruthy();
    expect(qwencloudCoding).toBeTruthy();
    expect(qwencloudToken).toBeTruthy();
  });

  it("includes 火山 plans and BytePlus", () => {
    const agentPlan = PROVIDER_PRESETS.find((p) => p.name === "火山 Agent Plan");
    const codingPlan = PROVIDER_PRESETS.find((p) => p.name === "火山 Coding Plan");
    const bytePlus = PROVIDER_PRESETS.find((p) => p.name === "BytePlus");
    expect(agentPlan).toBeTruthy();
    expect(codingPlan).toBeTruthy();
    expect(bytePlus).toBeTruthy();
  });

  it("includes Tencent Token Plan variants", () => {
    const tencent = PROVIDER_PRESETS.find((p) => p.name === "Tencent Token Plan");
    const tencentIntl = PROVIDER_PRESETS.find((p) => p.name === "Tencent Token Plan (Intl)");
    expect(tencent).toBeTruthy();
    expect(tencentIntl).toBeTruthy();
    expect(tencent?.modelsUrl).toBeTruthy();
  });

  it("includes Baidu Qianfan Coding and Token Plan", () => {
    const coding = PROVIDER_PRESETS.find((p) => p.name === "Baidu Qianfan Coding Plan");
    const token = PROVIDER_PRESETS.find((p) => p.name === "Baidu Qianfan Token Plan");
    expect(coding).toBeTruthy();
    expect(token).toBeTruthy();
  });

  it("includes Xiaomi MiMo Token Plan (China)", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "Xiaomi MiMo Token Plan (China)");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("token-plan");
  });

  it("includes Zhipu GLM en", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "Zhipu GLM en");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("z.ai");
  });

  it("includes MiniMax en", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "MiniMax en");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("minimax.io");
  });

  it("includes StepFun en", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "StepFun en");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("stepfun.ai");
  });

  it("includes Compshare Coding Plan", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "Compshare Coding Plan");
    expect(preset).toBeTruthy();
    expect(preset?.baseUrl).toContain("cp.compshare");
  });

  it("includes AWS Bedrock variants", () => {
    const aksk = PROVIDER_PRESETS.find((p) => p.name === "AWS Bedrock (AKSK)");
    const apiKey = PROVIDER_PRESETS.find((p) => p.name === "AWS Bedrock (API Key)");
    expect(aksk).toBeTruthy();
    expect(apiKey).toBeTruthy();
    expect(aksk?.templateValues).toBeTruthy();
    expect(aksk?.templateValues?.AWS_REGION).toBeTruthy();
  });

  it("includes KAT-Coder with template values", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.name === "KAT-Coder");
    expect(preset).toBeTruthy();
    expect(preset?.templateValues).toBeTruthy();
    expect(preset?.templateValues?.ENDPOINT_ID).toBeTruthy();
    expect(preset?.baseUrl).toContain("${ENDPOINT_ID}");
  });
});
