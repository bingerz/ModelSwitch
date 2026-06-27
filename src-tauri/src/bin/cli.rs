use clap::{Parser, Subcommand};
use model_switch_lib::{config::AppConfig, run_gateway, start_gateway_services};

#[derive(Parser)]
#[command(
    name = "modelswitch",
    about = "LLM Smart Gateway & Quota Scheduler",
    version
)]
struct Cli {
    /// Path to config file (default: platform config dir)
    #[arg(long, global = true)]
    config: Option<String>,

    /// Log level: trace, debug, info, warn, error
    #[arg(long, global = true)]
    log_level: Option<String>,

    /// Log format: text or json
    #[arg(long, global = true, default_value = "text")]
    log_format: String,

    /// Increase verbosity (-v = debug, -vv = trace)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress output (-q = warn, -qq = error)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    quiet: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gateway server (default)
    Serve {
        /// Override gateway host
        #[arg(long)]
        host: Option<String>,

        /// Override gateway port
        #[arg(short, long)]
        port: Option<u16>,
    },

    /// Create a starter config file at the default path
    Init,

    /// Load and validate the config file
    Validate,

    /// List configured channels
    Channels,

    /// Check gateway health status
    Status {
        /// Gateway URL (default: http://127.0.0.1:8080)
        #[arg(short, long, default_value = "http://127.0.0.1:8080")]
        url: String,
    },

    /// Stop a running gateway via PID file
    Stop,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Install a panic hook that logs the panic with location and backtrace
    // before the default abort behavior. This ensures panics are visible
    // in structured logs rather than being silently swallowed by tokio.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string panic payload>");

        tracing::error!(
            panic.location = %location,
            panic.payload = %payload,
            "panic occurred in gateway task"
        );

        // Chain to the default hook for backtrace printing (if RUST_BACKTRACE is set)
        default_hook(info);
    }));

    // Determine effective log level
    let log_level = resolve_log_level(&cli);
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", &log_level);
    }

    // Initialize tracing subscriber before any subcommands
    init_tracing(&cli.log_format);

    match cli.command.unwrap_or(Commands::Serve {
        host: None,
        port: None,
    }) {
        Commands::Serve { host, port } => cmd_serve(cli.config.as_deref(), host, port).await,
        Commands::Init => cmd_init(),
        Commands::Validate => cmd_validate(cli.config.as_deref()),
        Commands::Channels => cmd_channels(cli.config.as_deref()),
        Commands::Status { url } => cmd_status(url).await,
        Commands::Stop => cmd_stop(),
    }
}

fn init_tracing(format: &str) {
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    match format {
        "json" => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .init();
        }
        _ => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }
}

fn resolve_log_level(cli: &Cli) -> String {
    if let Some(ref level) = cli.log_level {
        return level.clone();
    }
    match (cli.verbose, cli.quiet) {
        (0, 0) => "info".to_string(),
        (v, 0) if v >= 2 => "trace".to_string(),
        (1.., 0) => "debug".to_string(),
        (0, q) if q >= 2 => "error".to_string(),
        (0, 1..) => "warn".to_string(),
        _ => "info".to_string(),
    }
}

async fn cmd_serve(config_path: Option<&str>, host: Option<String>, port: Option<u16>) {
    let path = config_path.map(std::path::PathBuf::from);

    let mut handles = start_gateway_services(path);
    if let Some(host) = host {
        handles.host = host;
    }
    if let Some(port) = port {
        handles.port = port;
    }

    tracing::info!(
        "ModelSwitch CLI — gateway starting on {}:{}",
        handles.host,
        handles.port
    );

    run_gateway(handles).await;
}

fn cmd_init() {
    match AppConfig::config_path() {
        Ok(path) => {
            if path.exists() {
                eprintln!("Config already exists at {}", path.display());
                std::process::exit(1);
            }
            let config = AppConfig::default();
            match config.save() {
                Ok(()) => println!("Created config at {}", path.display()),
                Err(e) => {
                    eprintln!("Failed to write config: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Cannot determine config path: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_validate(config_path: Option<&str>) {
    let result = match config_path {
        Some(path) => AppConfig::load_from(std::path::PathBuf::from(path)),
        None => AppConfig::load(),
    };
    match result {
        Ok(config) => {
            println!("Config is valid");
            println!("  Gateway: {}:{}", config.gateway.host, config.gateway.port);
            println!(
                "  Channels: {} ({})",
                config.channels.len(),
                config.channels.iter().filter(|c| c.enabled).count()
            );
            println!("  Routing: {}", config.gateway.routing_strategy);
            println!(
                "  Health check: {}",
                if config.gateway.health_check_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            println!(
                "  Fallback chains: {}",
                config.gateway.model_fallbacks.len()
            );
        }
        Err(e) => {
            eprintln!("Config validation failed: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_channels(config_path: Option<&str>) {
    let result = match config_path {
        Some(path) => AppConfig::load_from(std::path::PathBuf::from(path)),
        None => AppConfig::load(),
    };
    match result {
        Ok(config) => {
            if config.channels.is_empty() {
                println!("No channels configured.");
                return;
            }
            println!(
                "{:<20} {:<12} {:<6} {:<8} {:<8} NAME",
                "ID", "PROVIDER", "PRI", "WEIGHT", "ENABLED"
            );
            for ch in &config.channels {
                println!(
                    "{:<20} {:<12} {:<6} {:<8} {:<8} {}",
                    ch.id,
                    ch.provider,
                    ch.priority,
                    ch.weight,
                    if ch.enabled { "yes" } else { "no" },
                    ch.name,
                );
            }
        }
        Err(e) => {
            eprintln!("Failed to load config: {e}");
            std::process::exit(1);
        }
    }
}

async fn cmd_status(url: String) {
    let health_url = format!("{}/health", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("Failed to build HTTP client");

    match client.get(&health_url).send().await {
        Ok(resp) => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let status = body
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                let uptime = body
                    .get("uptime_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let channels = body.get("channels");
                let total = channels
                    .as_ref()
                    .and_then(|c| c.get("total"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let healthy = channels
                    .as_ref()
                    .and_then(|c| c.get("healthy"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                println!("Gateway is running");
                println!("  Status:  {}", status);
                println!("  Version: {}", version);
                println!("  Uptime:  {}s", uptime);
                println!("  Channels: {}/{} healthy", healthy, total);
                std::process::exit(0);
            } else {
                eprintln!("Gateway responded but returned invalid JSON");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Gateway unreachable at {}: {}", url, e);
            std::process::exit(1);
        }
    }
}

fn cmd_stop() {
    let pid_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch")
        .join("gateway.pid");

    if !pid_path.exists() {
        eprintln!("No PID file found at {}", pid_path.display());
        eprintln!("Is the gateway running?");
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            eprintln!("Invalid PID file content: {}", pid_str.trim());
            std::process::exit(1);
        }
    };

    #[cfg(unix)]
    {
        match unsafe { libc::kill(pid as i32, libc::SIGTERM) } {
            0 => {
                println!("Sent SIGTERM to process {}", pid);
                let _ = std::fs::remove_file(&pid_path);
            }
            _ => {
                eprintln!(
                    "Failed to send SIGTERM to process {} (not running or no permission)",
                    pid
                );
                let _ = std::fs::remove_file(&pid_path);
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(unix))]
    {
        eprintln!("Stop command is only supported on Unix systems");
        std::process::exit(1);
    }
}
