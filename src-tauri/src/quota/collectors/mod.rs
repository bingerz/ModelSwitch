pub mod deepseek;
pub mod jsonpath_generic;
pub mod minimax;
pub mod moonshot;
pub mod novita;
pub mod openai_compat;
pub mod openrouter;
pub mod response_header;
pub mod shengsuanyun;
pub mod siliconflow;
pub mod stepfun;
pub mod zhipu;

#[cfg(feature = "tauri")]
pub mod webview;

#[cfg(feature = "tauri")]
pub mod webview_scripts;

use crate::quota::provider::QuotaProvider;

/// Build the default registry with all built-in collectors.
pub fn default_registry() -> Vec<Box<dyn QuotaProvider>> {
    vec![
        // JSONPath collector first: when quota_config has balance_url + balance_path,
        // it takes priority over any other strategy
        Box::new(jsonpath_generic::JsonPathCollector),
        Box::new(openrouter::OpenRouterCollector),
        Box::new(deepseek::DeepSeekCollector),
        Box::new(siliconflow::SiliconFlowCollector),
        Box::new(moonshot::MoonshotCollector),
        Box::new(stepfun::StepFunCollector),
        Box::new(novita::NovitaCollector),
        Box::new(shengsuanyun::ShengSuanYunCollector),
        Box::new(zhipu::ZhipuCollector),
        Box::new(minimax::MiniMaxCollector),
        Box::new(openai_compat::OpenAiCompatCollector),
    ]
}
