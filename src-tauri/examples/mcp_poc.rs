//! MCP compile-validation PoC
//!
//! Purpose: prove that `rmcp` v1.7 can be added to ModelSwitch's axum 0.7 /
//! tokio stack without dependency conflicts. Success = it compiles.
//!
//! This is NOT production code. It spins up an MCP stdio server as a child
//! process, lists its tools, and prints them. If no `npx` is available the
//! runtime portion is skipped via a runtime check, but compilation must
//! succeed regardless.

use std::process::Stdio;

use anyhow::Result;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use tokio::process::Command;

#[tokio::main]
async fn main() -> Result<()> {
    println!("[mcp_poc] rmcp compile-validation PoC starting");

    // --- Step 1: spawn an MCP stdio server as a child process ------------
    // Use `npx` to run a known MCP server if available; otherwise skip the
    // runtime portion (compile success is the real goal).
    let npx_available = std::process::Command::new("which")
        .arg("npx")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !npx_available {
        println!("[mcp_poc] npx not found - skipping runtime portion");
        println!("[mcp_poc] compile validation PASSED (types resolved, no conflicts)");
        return Ok(());
    }

    let mut cmd = Command::new("npx");
    cmd.arg("-y")
        .arg("@modelcontextprotocol/server-everything")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    // TokioChildProcess::new takes Command by value (impl Into<CommandWrap>)
    let child = TokioChildProcess::new(cmd)?;
    println!("[mcp_poc] spawned child process transport");

    // --- Step 2: start the MCP client service ----------------------------
    // ().serve(transport) uses ServiceExt for the client role.
    let client = ().serve(child).await?;
    println!("[mcp_poc] client service started");

    // --- Step 3: list tools ----------------------------------------------
    let tools = client.peer().list_tools(Default::default()).await?;
    println!("[mcp_poc] server exposes {} tool(s):", tools.tools.len());
    for tool in &tools.tools {
        let desc = tool.description.as_deref().unwrap_or("(no desc)");
        println!("  - {} : {}", tool.name, desc);
    }

    // --- Step 4: call a tool (best-effort) -------------------------------
    // The "everything" server has an `echo` tool.
    // CallToolRequestParams is #[non_exhaustive], so we build via Default + mutation.
    let mut echo_params = CallToolRequestParams::default();
    echo_params.name = "echo".into();
    echo_params.arguments = serde_json::from_str(r#"{"message":"hello from rmcp PoC"}"#)?;
    let result = client.peer().call_tool(echo_params).await?;
    println!(
        "[mcp_poc] call_tool(echo) -> is_error={}",
        result.is_error.unwrap_or(false)
    );

    // --- Step 5: graceful shutdown ---------------------------------------
    client.cancel().await?;
    println!("[mcp_poc] done - PoC complete");
    Ok(())
}
