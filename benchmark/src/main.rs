use clap::{Parser, Subcommand};
use llm_gateway_bench::client::Protocol;
use llm_gateway_bench::mock::server::{self, MockConfig};
use llm_gateway_bench::report::{compare as compare_report, markdown};
use llm_gateway_bench::runner::workload::{self, WorkloadConfig};

/// LLM Gateway Benchmark Tool
#[derive(Parser, Debug)]
#[command(name = "gateway-bench", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // ── All the regular benchmark args ──
    /// Target gateway URL (required unless --mock-upstream)
    #[arg(long, default_value = "")]
    target: String,

    /// Test scenario: chat / streaming / mixed / burst / sustained
    #[arg(long, default_value = "chat")]
    scenario: String,

    /// Concurrent connections
    #[arg(long, default_value_t = 10)]
    concurrency: usize,

    /// Test duration in seconds
    #[arg(long, default_value_t = 30)]
    duration: u64,

    /// Warmup time in seconds (not counted in stats)
    #[arg(long, default_value_t = 5)]
    warmup: u64,

    /// Target RPS (None = unlimited)
    #[arg(long)]
    rps: Option<u32>,

    /// API key
    #[arg(long, default_value = "dummy")]
    api_key: String,

    /// Model name
    #[arg(long, default_value = "gpt-4")]
    model: String,

    /// Start mock LLM server as upstream
    #[arg(long)]
    mock_upstream: bool,

    /// Mock server port (0 = random)
    #[arg(long, default_value_t = 0)]
    mock_port: u16,

    /// Mock LLM processing delay in ms
    #[arg(long, default_value_t = 200)]
    mock_delay: u64,

    /// Mock 429 fail rate percentage
    #[arg(long, default_value_t = 0)]
    mock_fail_rate: u8,

    /// Mock response token count
    #[arg(long, default_value_t = 100)]
    mock_tokens: usize,

    /// Mock streaming chunk count (0 = derive from tokens)
    #[arg(long, default_value_t = 0)]
    mock_stream_chunks: usize,

    /// Timeout in seconds
    #[arg(long, default_value_t = 120)]
    timeout: u64,

    /// Generated message length (tokens)
    #[arg(long, default_value_t = 10)]
    message_tokens: usize,

    /// Burst size (for burst scenario)
    #[arg(long, default_value_t = 200)]
    burst_size: usize,

    /// Number of repeated runs for variance analysis
    #[arg(long, default_value_t = 1)]
    runs: usize,

    /// Mixed scenario stream ratio percentage (0-100)
    #[arg(long, default_value_t = 70)]
    mix_ratio: u8,

    /// Force streaming for all scenarios
    #[arg(long)]
    stream: bool,

    /// Protocol format: openai / anthropic / gemini
    #[arg(long, default_value = "openai")]
    protocol: String,

    /// Pool max idle connections per host
    #[arg(long, default_value_t = 100)]
    pool_max_idle: usize,

    /// Skip TLS certificate verification
    #[arg(long)]
    tls_skip_verify: bool,

    /// Report outputs: e.g. "json:results.json" or "markdown:results.md"
    #[arg(long, value_delimiter = ',')]
    report: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Compare direct vs proxy latency overhead
    Compare {
        /// Direct upstream URL
        #[arg(long)]
        direct: String,

        /// Proxy gateway URL
        #[arg(long)]
        proxy: String,

        /// API key
        #[arg(long, default_value = "dummy")]
        api_key: String,

        /// Model name
        #[arg(long, default_value = "gpt-4")]
        model: String,

        /// Test scenario
        #[arg(long, default_value = "chat")]
        scenario: String,

        /// Concurrent connections
        #[arg(long, default_value_t = 10)]
        concurrency: usize,

        /// Duration in seconds per phase
        #[arg(long, default_value_t = 30)]
        duration: u64,

        /// Warmup seconds
        #[arg(long, default_value_t = 5)]
        warmup: u64,

        /// Mock tokens
        #[arg(long, default_value_t = 100)]
        mock_tokens: usize,

        /// Report output path (markdown or json)
        #[arg(long)]
        report: Option<String>,
    },

    /// Start a standalone mock LLM server (blocks until Ctrl+C)
    MockServer {
        /// Mock server port (0 = random)
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Mock LLM processing delay in ms
        #[arg(long, default_value_t = 200)]
        delay: u64,

        /// Mock 429 fail rate percentage
        #[arg(long, default_value_t = 0)]
        fail_rate: u8,

        /// Mock response token count
        #[arg(long, default_value_t = 100)]
        tokens: usize,

        /// Mock streaming chunk count (0 = derive from tokens)
        #[arg(long, default_value_t = 0)]
        stream_chunks: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Validate CLI args (only for regular benchmark mode)
    if cli.command.is_none() {
        if cli.concurrency == 0 {
            anyhow::bail!("--concurrency must be at least 1");
        }
        if let Some(rps) = cli.rps {
            if rps == 0 {
                anyhow::bail!("--rps must be at least 1");
            }
        }
        if !cli.mock_upstream && cli.target.is_empty() {
            anyhow::bail!("--target <URL> is required when not using --mock-upstream");
        }
    }

    // Handle subcommands before regular benchmark flow
    match cli.command {
        Some(Commands::Compare {
            direct,
            proxy,
            api_key,
            model,
            scenario,
            concurrency,
            duration,
            warmup,
            mock_tokens,
            report,
        }) => {
            return run_compare(
                &direct,
                &proxy,
                &api_key,
                &model,
                &scenario,
                concurrency,
                duration,
                warmup,
                mock_tokens,
                report.as_deref(),
            )
            .await;
        }
        Some(Commands::MockServer {
            port,
            delay,
            fail_rate,
            tokens,
            stream_chunks,
        }) => {
            let mock_config = MockConfig {
                delay_ms: delay,
                fail_rate_pct: fail_rate,
                tokens,
                port,
                stream_chunks: if stream_chunks > 0 {
                    Some(stream_chunks)
                } else {
                    None
                },
            };
            let (server, bound_port) = server::start(mock_config).await?;
            tracing::info!(
                "Mock LLM server running on http://127.0.0.1:{} — press Ctrl+C to stop",
                bound_port
            );
            tokio::signal::ctrl_c().await?;
            tracing::info!("Shutting down mock server...");
            server.shutdown();
            return Ok(());
        }
        None => {}
    }

    // ── Regular benchmark mode ──
    let (target, _mock_server) = if cli.mock_upstream {
        let mock_config = MockConfig {
            delay_ms: cli.mock_delay,
            fail_rate_pct: cli.mock_fail_rate,
            tokens: cli.mock_tokens,
            port: cli.mock_port,
            stream_chunks: if cli.mock_stream_chunks > 0 {
                Some(cli.mock_stream_chunks)
            } else {
                None
            },
        };
        let (server, port) = server::start(mock_config).await?;
        let target = format!("http://127.0.0.1:{}", port);
        tracing::info!("Mock upstream on port {} — target overridden", port);
        (target, Some(server))
    } else {
        (cli.target.clone(), None)
    };

    // Override scenario if --stream is set
    let scenario = if cli.stream {
        "streaming".to_string()
    } else {
        cli.scenario.clone()
    };

    let workload_config = WorkloadConfig {
        target,
        api_key: cli.api_key,
        model: cli.model,
        scenario,
        concurrency: cli.concurrency,
        duration_secs: cli.duration,
        warmup_secs: cli.warmup,
        rps: cli.rps,
        message_tokens: cli.message_tokens,
        max_tokens: cli.mock_tokens,
        timeout_secs: cli.timeout,
        pool_max_idle: cli.pool_max_idle,
        tls_skip_verify: cli.tls_skip_verify,
        burst_size: cli.burst_size,
        runs: cli.runs,
        stream_ratio: cli.mix_ratio,
        protocol: Protocol::parse_str(&cli.protocol),
    };

    let summaries = workload::run_benchmark(workload_config).await?;
    let last = summaries.last().expect("at least one summary");
    println!("{}", markdown::format_summary(last));

    for spec in &cli.report {
        let parts: Vec<&str> = spec.splitn(2, ':').collect();
        if parts.len() != 2 {
            continue;
        }
        match parts[0] {
            "markdown" | "md" => markdown::write_report(last, parts[1])?,
            "json" => {
                let json = if summaries.len() > 1 {
                    serde_json::to_string_pretty(&summaries)?
                } else {
                    serde_json::to_string_pretty(last)?
                };
                std::fs::write(parts[1], json)?;
                tracing::info!("JSON report written to {}", parts[1]);
            }
            _ => tracing::warn!("Unknown report format '{}'", parts[0]),
        }
    }

    if let Some(server) = _mock_server {
        server.shutdown();
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_compare(
    direct: &str,
    proxy: &str,
    api_key: &str,
    model: &str,
    scenario: &str,
    concurrency: usize,
    duration: u64,
    warmup: u64,
    mock_tokens: usize,
    report_path: Option<&str>,
) -> anyhow::Result<()> {
    let result = compare_report::run_comparison(
        direct,
        proxy,
        api_key,
        model,
        scenario,
        concurrency,
        duration,
        warmup,
        120,
        100,
        false,
        mock_tokens,
    )
    .await?;

    println!("{}", result.to_markdown());

    if let Some(path) = report_path {
        if path.ends_with(".json") {
            let json = serde_json::to_string_pretty(&result)?;
            std::fs::write(path, json)?;
        } else {
            std::fs::write(path, result.to_markdown())?;
        }
        tracing::info!("Comparison report written to {}", path);
    }

    Ok(())
}
