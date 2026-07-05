//! The long-running JSON-RPC host: the Rust core as a backend process for the
//! GUI shells (Electron / Flutter).
//!
//! Reads one JSON-RPC request per line from stdin and writes one JSON response
//! per line to stdout. Diagnostics go to stderr so they never corrupt the
//! protocol stream.
//!
//! Usage:  `cargo run -p na-host --bin host -- [workspace_dir]`
//! (workspace_dir defaults to `./novel-workspace`)

use na_common::Result;
use na_host::{handle_line, Engine};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    let workspace = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./novel-workspace".to_string());

    let engine = Engine::new(&workspace)?;
    eprintln!(
        "[na-host] ready — workspace={} tools={}",
        engine.workspace_root().display(),
        engine.registry.len()
    );
    eprintln!("[na-host] send one JSON-RPC request per line on stdin. Example:");
    eprintln!(r#"[na-host]   {{"jsonrpc":"2.0","id":1,"method":"list_tools"}}"#);

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if let Some(response) = handle_line(&engine, &line).await {
            stdout.write_all(response.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    eprintln!("[na-host] stdin closed, shutting down.");
    Ok(())
}
