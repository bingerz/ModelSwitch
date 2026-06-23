/**
 * Preset provider configurations for quick channel setup.
 * Data extracted from cc-switch (reference/cc-switch) and adapted for ModelSwitch.
 */

export type PresetCategory =
  | "official"
  | "cn_official"
  | "aggregator"
  | "cloud_provider"
  | "third_party";

export type ApiFormat = "openai" | "anthropic" | "gemini";

export interface ProviderPreset {
  name: string;
  category: PresetCategory;
  /** Maps to Provider enum in backend: openai | anthropic | deepseek | gemini | openrouter | custom */
  provider: string;
  baseUrl: string;
  defaultModel: string;
  /** Model name aliases: requested model → actual upstream model */
  modelMapping: Record<string, string>;
  /** Suggested priority: 1=free/subscription, 2=economy API, 3=official */
  priority: number;
  iconColor: string;
  /** URL where user can obtain an API key */
  apiKeyUrl?: string;
  /** Available models for this provider (for model selector UI) */
  models: string[];
  /** API protocol: determines which proxy route handles the channel */
  apiFormat?: ApiFormat;
  /** Alternate endpoint URLs for other API formats. Key is the apiFormat value. */
  endpoints?: Partial<Record<ApiFormat, string>>;
  /** Quota monitoring strategy: how to check balance/usage for this provider */
  quotaStrategy?: "http_api" | "openai_compat" | "webview" | "response_header" | "disabled";
  /** Custom billing endpoint override (only for http_api strategy) */
  quotaEndpoint?: string;
}


export const PROVIDER_PRESETS: ProviderPreset[] = [
  // ─── Official ───
  {
    name: "Anthropic",
    category: "official",
    provider: "anthropic",
    baseUrl: "https://api.anthropic.com",
    defaultModel: "claude-sonnet-4-20250514",
    modelMapping: {},
    priority: 3,
    iconColor: "#D97757",
    apiKeyUrl: "https://console.anthropic.com/settings/keys",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    apiFormat: "anthropic",
    quotaStrategy: "webview", // console.anthropic.com/api/oauth/usage
  },
  {
    name: "OpenAI",
    category: "official",
    provider: "openai",
    baseUrl: "https://api.openai.com",
    defaultModel: "gpt-4o",
    modelMapping: {},
    priority: 3,
    iconColor: "#00A67E",
    apiKeyUrl: "https://platform.openai.com/api-keys",
    models: ["gpt-4o", "gpt-4o-mini", "o3", "o3-mini", "o4-mini"],
    quotaStrategy: "response_header",
  },

  // ─── CN Domestic ───
  {
    name: "DeepSeek",
    category: "cn_official",
    provider: "deepseek",
    baseUrl: "https://api.deepseek.com",
    defaultModel: "deepseek-v4-pro",
    modelMapping: {
      "gpt-4o": "deepseek-v4-pro",
      "gpt-4": "deepseek-v4-pro",
      "claude-3-sonnet": "deepseek-v4-pro",
    },
    priority: 2,
    iconColor: "#1E88E5",
    apiKeyUrl: "https://platform.deepseek.com/api_keys",
    models: ["deepseek-v4-pro", "deepseek-v4-flash"],
    quotaStrategy: "http_api", // GET /user/balance
    endpoints: {
      openai: "https://api.deepseek.com",
      anthropic: "https://api.deepseek.com/anthropic",
    },
  },
  {
    name: "Zhipu GLM",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://open.bigmodel.cn/api/paas",
    defaultModel: "glm-5",
    modelMapping: {
      "gpt-4o": "glm-5",
      "gpt-4": "glm-5",
    },
    priority: 2,
    iconColor: "#0F62FE",
    apiKeyUrl: "https://open.bigmodel.cn/usercenter/apikeys",
    models: ["glm-5"],
    quotaStrategy: "http_api", // GET /api/paas/api/biz/tokenAccounts/list
    endpoints: {
      openai: "https://open.bigmodel.cn/api/paas/v4",
      anthropic: "https://open.bigmodel.cn/api/anthropic",
    },
  },
  {
    name: "Baidu Qianfan",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://qianfan.baidubce.com",
    defaultModel: "ernie-4.0-8k",
    modelMapping: {
      "gpt-4o": "ernie-4.0-8k",
      "gpt-4": "ernie-4.0-8k",
    },
    priority: 2,
    iconColor: "#2932E1",
    apiKeyUrl: "https://console.bce.baidu.com/qianfan/ais/console/applicationConsole/application",
    models: ["ernie-4.0-8k"],
    quotaStrategy: "webview", // BCE HMAC required, fallback to WebView
  },
  {
    name: "Bailian (Alibaba)",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode",
    defaultModel: "qwen3-coder-plus",
    modelMapping: {
      "gpt-4o": "qwen3-coder-plus",
      "gpt-4": "qwen3-coder-plus",
      "claude-3-sonnet": "qwen3-coder-plus",
    },
    priority: 2,
    iconColor: "#624AFF",
    apiKeyUrl: "https://bailian.console.aliyun.com",
    models: ["qwen3-coder-plus", "qwen-max"],
    quotaStrategy: "webview", // Aliyun BSS HMAC required, fallback to WebView
    endpoints: {
      openai: "https://dashscope.aliyuncs.com/compatible-mode",
      anthropic: "https://dashscope.aliyuncs.com/compatible-mode/anthropic",
    },
  },
  {
    name: "Kimi (Moonshot)",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://api.moonshot.cn",
    defaultModel: "kimi-k2.6",
    modelMapping: {
      "gpt-4o": "kimi-k2.6",
      "gpt-4": "kimi-k2.6",
    },
    priority: 2,
    iconColor: "#6366F1",
    apiKeyUrl: "https://platform.moonshot.cn/console/api-keys",
    models: ["kimi-k2.6"],
    quotaStrategy: "http_api", // GET /v1/users/me/balance
    endpoints: {
      openai: "https://api.moonshot.cn/v1",
      anthropic: "https://api.moonshot.cn/anthropic",
    },
  },
  {
    name: "StepFun",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://api.stepfun.com/step_plan",
    defaultModel: "step-3.5-flash-2603",
    modelMapping: {
      "gpt-4o": "step-3.5-flash-2603",
      "gpt-4": "step-3.5-flash-2603",
    },
    priority: 2,
    iconColor: "#16D6D2",
    apiKeyUrl: "https://platform.stepfun.com/interface-key",
    models: ["step-3.5-flash-2603"],
    quotaStrategy: "http_api", // GET /v1/accounts
    endpoints: {
      openai: "https://api.stepfun.com",
      anthropic: "https://api.stepfun.com/anthropic",
    },
  },
  {
    name: "MiniMax",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://api.minimaxi.com",
    defaultModel: "MiniMax-M2.7",
    modelMapping: {
      "gpt-4o": "MiniMax-M2.7",
      "gpt-4": "MiniMax-M2.7",
    },
    priority: 2,
    iconColor: "#FF6B6B",
    apiKeyUrl: "https://platform.minimaxi.com",
    models: ["MiniMax-M2.7"],
    quotaStrategy: "http_api", // GET /v1/api/openplatform/coding_plan/remains
    endpoints: {
      openai: "https://api.minimaxi.com",
      anthropic: "https://api.minimaxi.com/anthropic",
    },
  },
  {
    name: "DouBao (ByteDance)",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://ark.cn-beijing.volces.com/api/coding",
    defaultModel: "doubao-seed-2-0-code-preview-latest",
    modelMapping: {
      "gpt-4o": "doubao-seed-2-0-code-preview-latest",
      "gpt-4": "doubao-seed-2-0-code-preview-latest",
    },
    priority: 2,
    iconColor: "#3370FF",
    apiKeyUrl: "https://www.volcengine.com/product/doubao",
    models: ["doubao-seed-2-0-code-preview-latest"],
    quotaStrategy: "webview", // Volcengine HMAC required
  },
  {
    name: "BaiLing (Alipay)",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://api.tbox.cn/api/anthropic",
    defaultModel: "Ling-2.5-1T",
    modelMapping: {
      "gpt-4o": "Ling-2.5-1T",
      "gpt-4": "Ling-2.5-1T",
    },
    priority: 2,
    iconColor: "#1677FF",
    models: ["Ling-2.5-1T"],
    quotaStrategy: "openai_compat",
    endpoints: {
      openai: "https://api.tbox.cn",
      anthropic: "https://api.tbox.cn/api/anthropic",
    },
  },
  {
    name: "Xiaomi MiMo",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://api.xiaomimimo.com/anthropic",
    defaultModel: "mimo-v2-pro",
    modelMapping: {
      "gpt-4o": "mimo-v2-pro",
      "gpt-4": "mimo-v2-pro",
    },
    priority: 2,
    iconColor: "#FF6900",
    models: ["mimo-v2-pro"],
    quotaStrategy: "openai_compat",
    endpoints: {
      openai: "https://api.xiaomimimo.com",
      anthropic: "https://api.xiaomimimo.com/anthropic",
    },
  },
  {
    name: "Longcat",
    category: "cn_official",
    provider: "custom",
    baseUrl: "https://api.longcat.chat/anthropic",
    defaultModel: "LongCat-Flash-Chat",
    modelMapping: {},
    priority: 2,
    iconColor: "#29E154",
    apiKeyUrl: "https://longcat.chat/platform/api_keys",
    models: ["LongCat-Flash-Chat"],
    quotaStrategy: "openai_compat",
    endpoints: {
      openai: "https://api.longcat.chat",
      anthropic: "https://api.longcat.chat/anthropic",
    },
  },

  // ─── Aggregators ───
  {
    name: "OpenRouter",
    category: "aggregator",
    provider: "openrouter",
    baseUrl: "https://openrouter.ai/api",
    defaultModel: "anthropic/claude-sonnet-4-20250514",
    modelMapping: {
      "gpt-4o": "openai/gpt-4o",
      "claude-3-sonnet": "anthropic/claude-sonnet-4-20250514",
    },
    priority: 2,
    iconColor: "#6D28D9",
    apiKeyUrl: "https://openrouter.ai/keys",
    models: [
      "anthropic/claude-sonnet-4-20250514",
      "anthropic/claude-opus-4-7",
      "anthropic/claude-haiku-4-5-20251001",
      "openai/gpt-4o",
      "openai/gpt-5.4",
      "google/gemini-3.1-pro",
    ],
    quotaStrategy: "http_api", // GET /api/v1/key
  },
  {
    name: "SiliconFlow",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://api.siliconflow.cn",
    defaultModel: "Pro/MiniMaxAI/MiniMax-M2.7",
    modelMapping: {
      "gpt-4o": "Pro/MiniMaxAI/MiniMax-M2.7",
      "gpt-4": "Pro/MiniMaxAI/MiniMax-M2.7",
    },
    priority: 2,
    iconColor: "#6E29F6",
    apiKeyUrl: "https://cloud.siliconflow.cn",
    models: ["Pro/MiniMaxAI/MiniMax-M2.7", "deepseek-ai/DeepSeek-V3"],
    quotaStrategy: "http_api", // GET /v1/user/info
    endpoints: {
      openai: "https://api.siliconflow.cn",
      anthropic: "https://api.siliconflow.cn/anthropic",
    },
  },
  {
    name: "AiHubMix",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://aihubmix.com",
    defaultModel: "gpt-4o",
    modelMapping: {},
    priority: 2,
    iconColor: "#006FFB",
    apiKeyUrl: "https://aihubmix.com",
    models: ["gpt-4o", "claude-sonnet-4-6", "claude-opus-4-7"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "DMXAPI",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://www.dmxapi.cn",
    defaultModel: "gpt-4o",
    modelMapping: {},
    priority: 2,
    iconColor: "#3B82F6",
    apiKeyUrl: "https://www.dmxapi.cn",
    models: ["gpt-4o", "claude-sonnet-4-6", "claude-opus-4-7"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "ModelScope",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://api-inference.modelscope.cn",
    defaultModel: "ZhipuAI/GLM-5",
    modelMapping: {
      "gpt-4o": "ZhipuAI/GLM-5",
      "gpt-4": "ZhipuAI/GLM-5",
    },
    priority: 2,
    iconColor: "#624AFF",
    models: ["ZhipuAI/GLM-5"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "TheRouter",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://api.therouter.ai",
    defaultModel: "anthropic/claude-sonnet-4-6",
    modelMapping: {
      "gpt-4o": "openai/gpt-5.4",
      "claude-3-sonnet": "anthropic/claude-sonnet-4-6",
    },
    priority: 2,
    iconColor: "#8B5CF6",
    models: [
      "anthropic/claude-sonnet-4-6",
      "anthropic/claude-opus-4-7",
      "openai/gpt-5.4",
      "google/gemini-3-flash-preview",
    ],
    quotaStrategy: "http_api", // credits API
  },
  {
    name: "Compshare",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://api.modelverse.cn",
    defaultModel: "gpt-4o",
    modelMapping: {},
    priority: 2,
    iconColor: "#0EA5E9",
    models: ["gpt-4o", "gpt-5.4"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "Shengsuanyun",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://router.shengsuanyun.com/api",
    defaultModel: "gpt-4o",
    modelMapping: {},
    priority: 2,
    iconColor: "#F59E0B",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "gpt-4o"],
    quotaStrategy: "http_api", // GET /api/v1/key
  },
  {
    name: "Novita AI",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://api.novita.ai",
    defaultModel: "deepseek/deepseek-r1",
    modelMapping: {},
    priority: 2,
    iconColor: "#10B981",
    apiKeyUrl: "https://novita.ai",
    models: ["deepseek/deepseek-r1", "zai-org/glm-5"],
    quotaStrategy: "http_api", // GET /openapi/v1/user/balance
    endpoints: {
      openai: "https://api.novita.ai",
      anthropic: "https://api.novita.ai/anthropic",
    },
  },
  {
    name: "PIPELLM",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://cc-api.pipellm.ai",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#4F46E5",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "response_header",
  },
  {
    name: "Nvidia",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://integrate.api.nvidia.com",
    defaultModel: "moonshotai/kimi-k2.5",
    modelMapping: {
      "gpt-4o": "moonshotai/kimi-k2.5",
    },
    priority: 2,
    iconColor: "#76B900",
    models: ["moonshotai/kimi-k2.5"],
    quotaStrategy: "response_header", // no billing API
  },
  {
    name: "NewAPI",
    category: "aggregator",
    provider: "custom",
    baseUrl: "https://your-newapi-host.com",
    defaultModel: "claude-sonnet-4-6",
    modelMapping: {},
    priority: 2,
    iconColor: "#6B7280",
    models: ["claude-sonnet-4-6", "gpt-4o", "gemini-3.1-pro"],
    quotaStrategy: "openai_compat",
  },

  // ─── Cloud Providers ───
  {
    name: "Azure OpenAI",
    category: "cloud_provider",
    provider: "openai",
    baseUrl: "https://YOUR_RESOURCE.openai.azure.com",
    defaultModel: "gpt-4o",
    modelMapping: {},
    priority: 3,
    iconColor: "#0078D4",
    apiKeyUrl: "https://oai.azure.com",
    models: ["gpt-4o", "gpt-4o-mini", "o3"],
    quotaStrategy: "response_header",
  },
  {
    name: "AWS Bedrock",
    category: "cloud_provider",
    provider: "anthropic",
    baseUrl: "https://bedrock-runtime.us-east-1.amazonaws.com",
    defaultModel: "anthropic.claude-sonnet-4-20250514-v1:0",
    modelMapping: {},
    priority: 3,
    iconColor: "#FF9900",
    models: ["anthropic.claude-opus-4-7-v1:0", "anthropic.claude-sonnet-4-6-v1:0", "anthropic.claude-haiku-4-5-v1:0"],
    quotaStrategy: "disabled", // requires CloudWatch SDK
  },
  {
    name: "Google Gemini",
    category: "cloud_provider",
    provider: "gemini",
    baseUrl: "https://generativelanguage.googleapis.com",
    defaultModel: "gemini-2.5-pro",
    modelMapping: {
      "gpt-4o": "gemini-2.5-pro",
      "gpt-4": "gemini-2.5-pro",
    },
    priority: 3,
    iconColor: "#4285F4",
    apiKeyUrl: "https://aistudio.google.com/app/apikey",
    models: ["gemini-2.5-pro", "gemini-2.5-flash", "gemini-3.1-pro"],
    apiFormat: "gemini",
    quotaStrategy: "response_header", // no billing API, only usageMetadata
  },

  // ─── Third-party Relay ───
  // All third-party relays typically use NewAPI/OneAPI and support openai_compat billing
  {
    name: "PackyCode",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://www.packyapi.com",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#6366F1",
    apiKeyUrl: "https://www.packyapi.com",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "Cubence",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://api.cubence.com",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#000000",
    apiKeyUrl: "https://cubence.com",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "AIGoCode",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://api.aigocode.com",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#5B7FFF",
    apiKeyUrl: "https://aigocode.com",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "RightCode",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://www.right.codes/claude",
    defaultModel: "claude-sonnet-4-6",
    modelMapping: {},
    priority: 2,
    iconColor: "#E96B2C",
    models: ["claude-sonnet-4-6", "claude-opus-4-7"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "AICodeMirror",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://api.aicodemirror.com/api/claudecode",
    defaultModel: "claude-sonnet-4-6",
    modelMapping: {},
    priority: 2,
    iconColor: "#14B8A6",
    models: ["claude-opus-4-7", "claude-sonnet-4-6"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "AICoding",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://api.aicoding.sh",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#8B5CF6",
    apiKeyUrl: "https://aicoding.sh",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "CrazyRouter",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://crazyrouter.com",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#E11D48",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "SSSAiCode",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://node-hk.sssaicode.com/api",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#7C3AED",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
  },
  {
    name: "Micu",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://www.openclaudecode.cn",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#06B6D4",
    models: ["claude-opus-4-7", "claude-sonnet-4-6"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "CTok.ai",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://api.ctok.ai",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#F97316",
    models: ["claude-opus-4-7", "claude-sonnet-4-6"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "DDSHub",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://www.ddshub.cc",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#EF4444",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "E-FlowCode",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://e-flowcode.cc",
    defaultModel: "gpt-5.4",
    modelMapping: {},
    priority: 2,
    iconColor: "#22C55E",
    models: ["gpt-5.4", "gpt-5.3-codex", "gpt-5.2"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "LionCCAPI",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://vibecodingapi.ai",
    defaultModel: "claude-opus-4-7",
    modelMapping: {},
    priority: 2,
    iconColor: "#A855F7",
    models: ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5-20251001"],
    quotaStrategy: "openai_compat",
  },
  {
    name: "LemonData",
    category: "third_party",
    provider: "custom",
    baseUrl: "https://api.lemondata.cc",
    defaultModel: "gpt-5.4",
    modelMapping: {},
    priority: 2,
    iconColor: "#FACC15",
    models: ["gpt-5.4"],
    quotaStrategy: "openai_compat",
  },
];

/** Group presets by category for the UI */
export function groupPresetsByCategory(): Record<PresetCategory, ProviderPreset[]> {
  const groups: Record<PresetCategory, ProviderPreset[]> = {
    official: [],
    cn_official: [],
    aggregator: [],
    cloud_provider: [],
    third_party: [],
  };
  for (const preset of PROVIDER_PRESETS) {
    groups[preset.category].push(preset);
  }
  return groups;
}
