use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use na_common::{CoreError, Json, Result};
use na_runtime::{CompletionCheck, LoopHook, Session};
use na_tools::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};

pub const TARGET_ENTRIES: usize = 8;
pub const TARGET_SOURCES: usize = 2;

#[derive(Debug, Default)]
pub struct FillProgress {
    pub steps: u32,
    pub sources: HashMap<String, String>,
    pub used_sources: HashSet<String>,
    pub added: usize,
    pub duplicates: usize,
}

#[derive(Debug, Default)]
pub struct FillContract(pub Mutex<FillProgress>);

impl LoopHook for FillContract {
    fn on_step_start(&self, step: u32, _session: &Session) {
        if let Ok(mut progress) = self.0.lock() {
            progress.steps = step;
        }
    }
}

impl CompletionCheck for FillContract {
    fn unmet_requirement(&self, _session: &Session) -> Option<String> {
        let Ok(progress) = self.0.lock() else {
            return Some("采集进度无法校验，不得声明成功。".to_string());
        };
        if progress.added >= TARGET_ENTRIES && progress.used_sources.len() >= TARGET_SOURCES {
            return None;
        }
        Some(format!(
            "采集尚未达标：实际新增 {} 条，已引用 {} 个来源。任务仍是研究指定作品并填充知识库，\
             不要反问用户任务或仅输出总结。请继续 web_fetch 取证并 knowledge_save 保存，\
             至少新增 {TARGET_ENTRIES} 条且引用 {TARGET_SOURCES} 个来源；不得编造资料凑数。\
             已取得的网页应先提取再保存，来源不可访问时换来源。",
            progress.added,
            progress.used_sources.len()
        ))
    }
}

pub struct EvidenceFetchTool {
    pub inner: Arc<dyn Tool>,
    pub contract: Arc<FillContract>,
}

impl Tool for EvidenceFetchTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let url = args
                .get("url")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing url"))?
                .trim()
                .to_string();
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err(CoreError::invalid_input("source must be an HTTP(S) URL"));
            }
            let mut result = self.inner.execute(args, ctx).await?;
            if !result.ok {
                return Ok(result);
            }
            // Bound each observation while preserving the exact text used for
            // quote validation. The session keeps the same excerpt.
            let excerpt: String = result.content.chars().take(6000).collect();
            if excerpt.trim().chars().count() < 40 {
                return Err(CoreError::invalid_input(
                    "网页没有足够的可读正文，请更换来源",
                ));
            }
            result.metadata.truncated |= excerpt.len() < result.content.len();
            self.contract
                .0
                .lock()
                .map_err(|_| CoreError::internal("采集进度锁已损坏"))?
                .sources
                .insert(url.clone(), excerpt.clone());
            result.content = format!("来源 URL: {url}\n\n{excerpt}");
            result.metadata.untrusted = true;
            Ok(result)
        })
    }
}

pub fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
