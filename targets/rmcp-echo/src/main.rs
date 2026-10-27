//! Minimal MCP server built on the official Rust SDK (`rmcp`).
//!
//! It exposes a single `echo` tool with the same contract as the `echo`
//! tool of mcp-server-rs (`{"text": string}` -> text content), so both
//! servers can be compared message by message over stdio.

use rmcp::{
    handler::server::wrapper::Parameters, schemars, tool, tool_handler, tool_router, ServerHandler,
    ServiceExt,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EchoParams {
    /// Text to return unchanged.
    text: String,
}

#[derive(Clone)]
struct Echo;

#[tool_router]
impl Echo {
    #[tool(description = "Returns the given text unchanged.")]
    fn echo(&self, Parameters(EchoParams { text }): Parameters<EchoParams>) -> String {
        text
    }
}

#[tool_handler(name = "rmcp-echo", version = "0.1.0")]
impl ServerHandler for Echo {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = Echo.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
