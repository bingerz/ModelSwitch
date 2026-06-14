//! Example: Automatically compare direct vs proxy overhead.
//!
//! ```bash
//! cargo run --example compare_overhead
//! ```

use llm_gateway_bench::report::compare;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let result = compare::run_comparison(
        "https://api.openai.com",
        "http://127.0.0.1:8080",
        &std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "dummy".into()),
        "gpt-4",
        "chat",
        50,    // concurrency
        30,    // duration per phase
        5,     // warmup
        120,   // timeout
        100,   // pool_max_idle
        false, // tls_skip_verify
        100,   // mock_tokens
    )
    .await?;

    println!("{}", result.to_markdown());

    Ok(())
}
