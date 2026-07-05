//! Tool-call scheduling with safe concurrency.
//!
//! A single model turn may request several tool calls at once. Running them all
//! sequentially is slow; running them all concurrently is unsafe because two
//! *mutating* tools could race on the workspace. [`ToolScheduler`] gets both
//! right:
//!
//! * **Read-only** tools run **concurrently** via a `tokio::task::JoinSet`, with
//!   a hard cap on in-flight calls.
//! * **Subagent** tools run concurrently in their own bounded phase, so several
//!   delegated child tasks can proceed without racing normal writes.
//! * **Mutating** tools run **serially**, *after* reads/subagents, in the order
//!   they were requested — so writes never race each other or read stale state.
//!
//! Every call still goes through [`ToolRegistry::invoke`](na_tools::ToolRegistry::invoke),
//! which already enforces validation, authorization, per-call timeout and
//! cancellation. The scheduler additionally watches the shared
//! [`CancellationToken`]: when it fires, pending (not-yet-started) work is
//! dropped and outstanding calls are aborted, so a user interrupt stops the batch
//! promptly. Results are returned keyed by each call's [`ToolCallId`].

use std::collections::HashMap;

use na_common::{CancellationToken, ToolCallId};
use na_tools::{ToolConcurrency, ToolContext, ToolRegistry, ToolResult};

use crate::message::ToolCallRequest;

/// Runs a batch of tool calls with safe concurrency and cancellation.
#[derive(Debug, Default, Clone)]
pub struct ToolScheduler;

const DEFAULT_MAX_PARALLEL_READS: usize = 8;
const DEFAULT_MAX_PARALLEL_SUBAGENTS: usize = 3;

impl ToolScheduler {
    /// Construct a scheduler.
    pub fn new() -> Self {
        ToolScheduler
    }

    /// Execute `calls` against `registry`/`ctx`, returning a map from each
    /// call's [`ToolCallId`] to its [`ToolResult`].
    ///
    /// Reads run concurrently; writes run serially afterwards in request order.
    /// If the context's cancellation token fires, not-yet-started calls are
    /// skipped and recorded with a cancelled error result, and in-flight reads
    /// are aborted (their `invoke` returns a cancelled result via the registry's
    /// own guard).
    pub async fn run_batch(
        &self,
        calls: &[ToolCallRequest],
        registry: &ToolRegistry,
        ctx: &ToolContext,
    ) -> HashMap<ToolCallId, ToolResult> {
        let mut results: HashMap<ToolCallId, ToolResult> = HashMap::new();
        if calls.is_empty() {
            return results;
        }

        // Partition into reads/subagents (bounded concurrent) and writes
        // (serial) preserving write order.
        // A call to an unknown tool is treated as a write (serialized) so its
        // NotFound result is produced deterministically; mutating defaults safe.
        let mut reads: Vec<&ToolCallRequest> = Vec::new();
        let mut subagents: Vec<&ToolCallRequest> = Vec::new();
        let mut writes: Vec<&ToolCallRequest> = Vec::new();
        for call in calls {
            match concurrency(call, registry) {
                ToolConcurrency::ReadOnly => reads.push(call),
                ToolConcurrency::Subagent => subagents.push(call),
                ToolConcurrency::Mutating => writes.push(call),
            }
        }

        // ---- Phase 1: run reads concurrently ----
        if !reads.is_empty() {
            self.run_concurrent(
                &reads,
                DEFAULT_MAX_PARALLEL_READS,
                "read",
                registry,
                ctx,
                &mut results,
            )
            .await;
        }

        // ---- Phase 2: run subagents concurrently, capped separately ----
        if !subagents.is_empty() {
            self.run_concurrent(
                &subagents,
                DEFAULT_MAX_PARALLEL_SUBAGENTS,
                "subagent",
                registry,
                ctx,
                &mut results,
            )
            .await;
        }

        // ---- Phase 3: run writes serially, in request order ----
        for call in writes {
            if ctx.cancel.is_cancelled() {
                results.insert(
                    call.id.clone(),
                    cancelled_result("scheduler: cancelled before mutating call"),
                );
                continue;
            }
            let res = registry.invoke(&call.name, call.args.clone(), ctx).await;
            results.insert(call.id.clone(), res);
        }

        results
    }

    /// Run a set of calls concurrently with a `JoinSet`, racing the
    /// cancellation token and limiting in-flight work.
    async fn run_concurrent(
        &self,
        calls: &[&ToolCallRequest],
        limit: usize,
        label: &str,
        registry: &ToolRegistry,
        ctx: &ToolContext,
        results: &mut HashMap<ToolCallId, ToolResult>,
    ) {
        let mut set: tokio::task::JoinSet<(ToolCallId, ToolResult)> = tokio::task::JoinSet::new();
        let limit = limit.max(1);
        let mut next = 0usize;

        while next < calls.len() || !set.is_empty() {
            while next < calls.len() && set.len() < limit {
                let call = calls[next];
                next += 1;
                // If already cancelled, do not even spawn — record and move on.
                if ctx.cancel.is_cancelled() {
                    results.insert(
                        call.id.clone(),
                        cancelled_result(&format!("scheduler: cancelled before {label} call")),
                    );
                    continue;
                }
                // Clone the cheap, shareable handles for the spawned task. The
                // registry is `Clone` (Arc map) and the context is `Clone` (Arc
                // stores), so each task owns what it needs.
                let registry = registry.clone();
                let ctx = ctx.clone();
                let id = call.id.clone();
                let name = call.name.clone();
                let args = call.args.clone();
                set.spawn(async move {
                    let res = registry.invoke(&name, args, &ctx).await;
                    (id, res)
                });
            }

            if set.is_empty() {
                continue;
            }

            let cancel: CancellationToken = ctx.cancel.clone();
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    set.abort_all();
                    while let Some(joined) = set.join_next().await {
                        if let Ok((id, res)) = joined {
                            results.entry(id).or_insert(res);
                        }
                    }
                    break;
                }
                joined = set.join_next() => {
                    match joined {
                        Some(Ok((id, res))) => {
                            results.insert(id, res);
                        }
                        Some(Err(_join_err)) => {}
                        None => {}
                    }
                }
            }
        }

        // Fill in any calls that never produced a result (aborted due to
        // cancellation) with a cancelled result, so the map is complete.
        for call in calls {
            results.entry(call.id.clone()).or_insert_with(|| {
                cancelled_result(&format!("scheduler: {label} call aborted by cancellation"))
            });
        }
    }
}

/// The scheduler class for a requested call. Unknown tools are treated as
/// mutating (serialized) so their deterministic NotFound result is produced in
/// the safe phase.
fn concurrency(call: &ToolCallRequest, registry: &ToolRegistry) -> ToolConcurrency {
    match registry.get(&call.name) {
        Some(tool) => {
            let spec = tool.spec();
            match spec.concurrency {
                ToolConcurrency::Subagent => ToolConcurrency::Subagent,
                ToolConcurrency::Mutating => ToolConcurrency::Mutating,
                ToolConcurrency::ReadOnly if spec.mutating => ToolConcurrency::Mutating,
                ToolConcurrency::ReadOnly => ToolConcurrency::ReadOnly,
            }
        }
        None => ToolConcurrency::Mutating,
    }
}

/// Build a cancelled error [`ToolResult`].
fn cancelled_result(msg: &str) -> ToolResult {
    ToolResult::from_error(&na_common::CoreError::cancelled(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::{json, CancellationToken};
    use na_tools::Result as TResult;
    use na_tools::{BoxFuture, Tool, ToolContext, ToolContextBuilder, ToolSpec};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_sched_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        p
    }

    /// A read-only tool that records concurrency and echoes its `id` arg.
    struct ReadTool {
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        delay_ms: u64,
    }
    impl Tool for ReadTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "reader",
                "read-only echo",
                json!({ "type": "object" }),
                vec![],
                false, // not mutating => concurrent
            )
        }
        fn execute<'a>(
            &'a self,
            args: na_common::Json,
            _ctx: &'a ToolContext,
        ) -> BoxFuture<'a, TResult<ToolResult>> {
            Box::pin(async move {
                let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
                self.live.fetch_sub(1, Ordering::SeqCst);
                let tag = args.get("tag").cloned().unwrap_or(json!("?"));
                Ok(ToolResult::success("read-ok", json!({ "tag": tag })))
            })
        }
    }

    /// A mutating tool that records the order of its invocations.
    struct WriteTool {
        order: Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl Tool for WriteTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "writer",
                "mutating",
                json!({ "type": "object" }),
                vec![], // no capability gate so permissive ctx allows it
                true,   // mutating => serialized
            )
        }
        fn execute<'a>(
            &'a self,
            args: na_common::Json,
            _ctx: &'a ToolContext,
        ) -> BoxFuture<'a, TResult<ToolResult>> {
            Box::pin(async move {
                let tag = args
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                self.order.lock().unwrap().push(tag.clone());
                Ok(ToolResult::success(
                    format!("wrote {tag}"),
                    json!({ "tag": tag }),
                ))
            })
        }
    }

    struct SubagentLikeTool {
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        order: Arc<std::sync::Mutex<Vec<String>>>,
        delay_ms: u64,
    }
    impl Tool for SubagentLikeTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "subagent_like",
                "subagent-like delegated work",
                json!({ "type": "object" }),
                vec![],
                true,
            )
            .with_subagent_concurrency()
        }
        fn execute<'a>(
            &'a self,
            args: na_common::Json,
            _ctx: &'a ToolContext,
        ) -> BoxFuture<'a, TResult<ToolResult>> {
            Box::pin(async move {
                let tag = args
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                self.order.lock().unwrap().push(format!("sub-start-{tag}"));
                let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
                self.live.fetch_sub(1, Ordering::SeqCst);
                self.order.lock().unwrap().push(format!("sub-end-{tag}"));
                Ok(ToolResult::success("sub-ok", json!({ "tag": tag })))
            })
        }
    }

    /// A read tool that blocks "forever" (to test cancellation).
    struct SlowRead;
    impl Tool for SlowRead {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "slow_read",
                "slow",
                json!({ "type": "object" }),
                vec![],
                false,
            )
        }
        fn execute<'a>(
            &'a self,
            _args: na_common::Json,
            _ctx: &'a ToolContext,
        ) -> BoxFuture<'a, TResult<ToolResult>> {
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                Ok(ToolResult::success("done", json!({})))
            })
        }
    }

    fn ctx(tag: &str, cancel: CancellationToken) -> ToolContext {
        ToolContextBuilder::new(temp_root(tag))
            .cancel(cancel)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn reads_run_concurrently() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadTool {
            live: live.clone(),
            peak: peak.clone(),
            delay_ms: 80,
        }))
        .unwrap();

        let c = ctx("reads", CancellationToken::new());
        let calls: Vec<ToolCallRequest> = (0..4)
            .map(|i| ToolCallRequest::new("reader", json!({ "tag": i })))
            .collect();

        let sched = ToolScheduler::new();
        let results = sched.run_batch(&calls, &reg, &c).await;
        assert_eq!(results.len(), 4);
        assert!(results.values().all(|r| r.ok));
        // Concurrency actually happened: more than one ran at once.
        assert!(peak.load(Ordering::SeqCst) > 1, "reads should overlap");
    }

    #[tokio::test]
    async fn reads_are_bounded() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadTool {
            live,
            peak: peak.clone(),
            delay_ms: 40,
        }))
        .unwrap();

        let c = ctx("reads_bound", CancellationToken::new());
        let calls: Vec<ToolCallRequest> = (0..20)
            .map(|i| ToolCallRequest::new("reader", json!({ "tag": i })))
            .collect();

        let results = ToolScheduler::new().run_batch(&calls, &reg, &c).await;
        assert_eq!(results.len(), 20);
        assert!(peak.load(Ordering::SeqCst) <= DEFAULT_MAX_PARALLEL_READS);
        assert!(peak.load(Ordering::SeqCst) > 1);
    }

    #[tokio::test]
    async fn writes_run_serially_in_order() {
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(WriteTool {
            order: order.clone(),
        }))
        .unwrap();

        let c = ctx("writes", CancellationToken::new());
        let calls: Vec<ToolCallRequest> = ["a", "b", "c"]
            .iter()
            .map(|t| ToolCallRequest::new("writer", json!({ "tag": t })))
            .collect();

        let sched = ToolScheduler::new();
        let results = sched.run_batch(&calls, &reg, &c).await;
        assert_eq!(results.len(), 3);
        // Serialized in request order.
        assert_eq!(*order.lock().unwrap(), vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn mixed_reads_and_writes_all_return() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadTool {
            live,
            peak,
            delay_ms: 10,
        }))
        .unwrap();
        reg.register(Arc::new(WriteTool {
            order: order.clone(),
        }))
        .unwrap();

        let c = ctx("mixed", CancellationToken::new());
        let calls = vec![
            ToolCallRequest::new("reader", json!({ "tag": "r1" })),
            ToolCallRequest::new("writer", json!({ "tag": "w1" })),
            ToolCallRequest::new("reader", json!({ "tag": "r2" })),
            ToolCallRequest::new("writer", json!({ "tag": "w2" })),
        ];
        let sched = ToolScheduler::new();
        let results = sched.run_batch(&calls, &reg, &c).await;
        // All four calls produced a result.
        assert_eq!(results.len(), 4);
        for call in &calls {
            assert!(results.contains_key(&call.id));
        }
        // Writes serialized in order.
        assert_eq!(*order.lock().unwrap(), vec!["w1", "w2"]);
    }

    #[tokio::test]
    async fn subagent_class_runs_concurrently_between_reads_and_writes() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let read_live = Arc::new(AtomicUsize::new(0));
        let read_peak = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadTool {
            live: read_live,
            peak: read_peak,
            delay_ms: 5,
        }))
        .unwrap();
        reg.register(Arc::new(SubagentLikeTool {
            live,
            peak: peak.clone(),
            order: order.clone(),
            delay_ms: 40,
        }))
        .unwrap();
        reg.register(Arc::new(WriteTool {
            order: order.clone(),
        }))
        .unwrap();

        let c = ctx("subagents", CancellationToken::new());
        let calls = vec![
            ToolCallRequest::new("reader", json!({ "tag": "r" })),
            ToolCallRequest::new("subagent_like", json!({ "tag": "a" })),
            ToolCallRequest::new("subagent_like", json!({ "tag": "b" })),
            ToolCallRequest::new("writer", json!({ "tag": "w" })),
        ];

        let results = ToolScheduler::new().run_batch(&calls, &reg, &c).await;
        assert_eq!(results.len(), 4);
        assert!(peak.load(Ordering::SeqCst) > 1, "subagents should overlap");
        let order = order.lock().unwrap().clone();
        let write_pos = order.iter().position(|x| x == "w").unwrap();
        let last_sub_end = order
            .iter()
            .rposition(|x| x.starts_with("sub-end-"))
            .unwrap();
        assert!(
            last_sub_end < write_pos,
            "serial writes should wait until subagents finish"
        );
    }

    #[tokio::test]
    async fn cancellation_stops_pending_calls() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(SlowRead)).unwrap();
        let cancel = CancellationToken::new();
        let c = ctx("cancel", cancel.clone());

        let calls: Vec<ToolCallRequest> = (0..3)
            .map(|_| ToolCallRequest::new("slow_read", json!({})))
            .collect();

        let sched = ToolScheduler::new();
        let fut = sched.run_batch(&calls, &reg, &c);
        // Cancel shortly after starting.
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            cancel.cancel();
        });

        let results = tokio::time::timeout(std::time::Duration::from_secs(5), fut)
            .await
            .expect("scheduler must return promptly after cancel");
        canceller.await.unwrap();

        // Every call has a result, and all are cancelled (not stuck).
        assert_eq!(results.len(), 3);
        assert!(results.values().all(|r| !r.ok));
        assert!(results.values().all(|r| r.data["code"] == "cancelled"));
    }

    #[tokio::test]
    async fn empty_batch_returns_empty() {
        let reg = ToolRegistry::new();
        let c = ctx("empty", CancellationToken::new());
        let results = ToolScheduler::new().run_batch(&[], &reg, &c).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn unknown_tool_yields_not_found_result() {
        let reg = ToolRegistry::new();
        let c = ctx("unknown", CancellationToken::new());
        let calls = vec![ToolCallRequest::new("nope", json!({}))];
        let results = ToolScheduler::new().run_batch(&calls, &reg, &c).await;
        assert_eq!(results.len(), 1);
        let r = results.values().next().unwrap();
        assert!(!r.ok);
        assert_eq!(r.data["code"], "not_found");
    }
}
