//! The long-running JSON-RPC host: the Rust core as a backend process for the
//! GUI shells (Electron / Flutter).
//!
//! Reads one JSON-RPC request per line from stdin and writes one JSON response
//! per line to stdout. Diagnostics go to stderr so they never corrupt the
//! protocol stream.
//!
//! Usage:  `cargo run -p na-host --bin host -- [workspace_dir]`
//! (workspace_dir defaults to `./novel-workspace`)

use std::sync::Arc;

use na_common::{json, CoreError, Json, Result};
use na_host::{handle_line, Engine, RpcRequest};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

const OPERATION_QUEUE_CAPACITY: usize = 64;

#[tokio::main]
async fn main() -> Result<()> {
    let workspace = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./novel-workspace".to_string());

    let engine = Arc::new(Engine::new(&workspace)?);
    eprintln!(
        "[na-host] ready — workspace={} tools={}",
        engine.workspace_root().display(),
        engine.registry.len()
    );
    eprintln!("[na-host] send one JSON-RPC request per line on stdin. Example:");
    eprintln!(r#"[na-host]   {{"jsonrpc":"2.0","id":1,"method":"list_tools"}}"#);

    serve(tokio::io::stdin(), tokio::io::stdout(), engine).await?;

    eprintln!("[na-host] stdin closed, shutting down.");
    Ok(())
}

/// Serve newline-delimited requests without blocking the input loop on a long
/// goal. Responses may complete out of order and are correlated by JSON-RPC id.
async fn serve<R, W>(reader: R, writer: W, engine: Arc<Engine>) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    let (response_tx, mut response_rx) = mpsc::channel::<String>(64);
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(response) = response_rx.recv().await {
            writer.write_all(response.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
        std::io::Result::Ok(())
    });

    let (operation_tx, mut operation_rx) = mpsc::channel::<String>(OPERATION_QUEUE_CAPACITY);
    let operation_engine = Arc::clone(&engine);
    let operation_responses = response_tx.clone();
    let operation_worker = tokio::spawn(async move {
        while let Some(line) = operation_rx.recv().await {
            if let Some(response) = handle_line(&operation_engine, &line).await {
                if operation_responses.send(response).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if is_control_request(&line) {
            if let Some(response) = handle_line(&engine, &line).await {
                response_tx
                    .send(response)
                    .await
                    .map_err(|_| CoreError::internal("RPC response writer stopped unexpectedly"))?;
            }
            continue;
        }

        match operation_tx.try_send(line) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(line)) => {
                response_tx
                    .send(busy_response(&line))
                    .await
                    .map_err(|_| CoreError::internal("RPC response writer stopped unexpectedly"))?;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(CoreError::internal(
                    "RPC operation worker stopped unexpectedly",
                ));
            }
        }
    }

    drop(operation_tx);
    operation_worker
        .await
        .map_err(|error| CoreError::internal(format!("RPC operation worker failed: {error}")))?;
    drop(response_tx);
    writer_task
        .await
        .map_err(|error| CoreError::internal(format!("RPC writer task failed: {error}")))??;
    Ok(())
}

/// Control requests are read-only or explicitly interrupt work, so they must
/// not queue behind a long operation. All other requests are serialized to
/// preserve tool/checkpoint/VCS ordering across independent RPC callers.
fn is_control_request(line: &str) -> bool {
    serde_json::from_str::<RpcRequest>(line)
        .map(|request| matches!(request.method.as_str(), "ping" | "cancel" | "list_tools"))
        .unwrap_or(false)
}

fn busy_response(line: &str) -> String {
    let id = serde_json::from_str::<RpcRequest>(line)
        .map(|request| request.id)
        .unwrap_or(Json::Null);
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32005,
            "message": "RPC server is busy; retry the request",
        }
    }))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::{json, Json};
    use na_tools::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::Notify;
    use tokio::time::{timeout, Duration};

    struct BlockingTool {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    impl Tool for BlockingTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "blocking_test",
                "test-only blocking tool",
                json!({}),
                vec![],
                false,
            )
        }

        fn execute<'a>(
            &'a self,
            _args: Json,
            _ctx: &'a ToolContext,
        ) -> BoxFuture<'a, Result<ToolResult>> {
            Box::pin(async move {
                self.started.notify_one();
                self.release.notified().await;
                Ok(ToolResult::success("released", Json::Null))
            })
        }
    }

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "na_host_concurrent_rpc_{}",
            na_common::next_id("t")
        ))
    }

    #[tokio::test]
    async fn fast_rpc_is_not_blocked_behind_long_request() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let mut engine = Engine::new(temp_root()).unwrap();
        engine
            .registry
            .register(Arc::new(BlockingTool {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            }))
            .unwrap();

        let (client, server) = duplex(4096);
        let (server_reader, server_writer) = tokio::io::split(server);
        let server_task = tokio::spawn(serve(server_reader, server_writer, Arc::new(engine)));
        let (client_reader, mut client_writer) = tokio::io::split(client);
        let mut responses = BufReader::new(client_reader).lines();

        client_writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"blocking_test\",\"params\":{}}\n",
            )
            .await
            .unwrap();
        timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("blocking request did not start");
        client_writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n")
            .await
            .unwrap();

        let first = timeout(Duration::from_secs(1), responses.next_line())
            .await
            .expect("ping was blocked behind the long request")
            .unwrap()
            .unwrap();
        let first: Json = serde_json::from_str(&first).unwrap();
        assert_eq!(first["id"], 2);
        assert_eq!(first["result"], "pong");

        client_writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"write_file\",\"params\":{\"path\":\"ordered.md\",\"content\":\"after\"}}\n",
            )
            .await
            .unwrap();
        assert!(
            timeout(Duration::from_millis(50), responses.next_line())
                .await
                .is_err(),
            "a second tool operation overtook the blocked request"
        );

        release.notify_one();
        let mut completed = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let response: Json = serde_json::from_str(
                &timeout(Duration::from_secs(1), responses.next_line())
                    .await
                    .expect("serialized operation response was not returned")
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(response["result"]["ok"], true);
            completed.insert(response["id"].as_i64().unwrap());
        }
        assert_eq!(completed, [1, 3].into_iter().collect());

        client_writer.shutdown().await.unwrap();
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn operation_queue_is_bounded_and_reports_overload() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let mut engine = Engine::new(temp_root()).unwrap();
        engine
            .registry
            .register(Arc::new(BlockingTool {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
            }))
            .unwrap();

        let (client, server) = duplex(128 * 1024);
        let (server_reader, server_writer) = tokio::io::split(server);
        let server_task = tokio::spawn(serve(server_reader, server_writer, Arc::new(engine)));
        let (client_reader, mut client_writer) = tokio::io::split(client);
        let mut responses = BufReader::new(client_reader).lines();

        client_writer
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"blocking_test\",\"params\":{}}\n",
            )
            .await
            .unwrap();
        timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("blocking request did not start");

        for id in 0..=OPERATION_QUEUE_CAPACITY {
            let request = format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"write_file\",\"params\":{{\"path\":\"queued-{id}.md\",\"content\":\"x\"}}}}\n",
                id + 10
            );
            client_writer.write_all(request.as_bytes()).await.unwrap();
        }

        let overloaded = timeout(Duration::from_secs(1), responses.next_line())
            .await
            .expect("queue saturation did not produce a response")
            .unwrap()
            .unwrap();
        let overloaded: Json = serde_json::from_str(&overloaded).unwrap();
        assert_eq!(overloaded["error"]["code"], -32005);

        client_writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"cancel\"}\n")
            .await
            .unwrap();
        let cancelled = timeout(Duration::from_secs(1), responses.next_line())
            .await
            .expect("control request was blocked by a full operation queue")
            .unwrap()
            .unwrap();
        let cancelled: Json = serde_json::from_str(&cancelled).unwrap();
        assert_eq!(cancelled["id"], 2);
        assert_eq!(cancelled["result"], "cancelled");

        release.notify_one();
        client_writer.shutdown().await.unwrap();
        server_task.await.unwrap().unwrap();
    }
}
