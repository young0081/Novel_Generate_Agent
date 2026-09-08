use na_common::{CoreError, Result};
use na_runtime::{Message, Session, SessionRecord, SessionStore};
use serde::{Deserialize, Serialize};

const KIND: &str = "knowledge_collection";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionRun {
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub status: String,
    pub added: usize,
    pub sources: usize,
    pub steps: u32,
    pub stopped_reason: String,
    pub error: Option<String>,
    pub follow_up: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionHistory {
    pub kb_id: String,
    pub topic: String,
    pub runs: Vec<CollectionRun>,
}

pub fn metadata(record: &SessionRecord) -> Result<CollectionHistory> {
    if record.kind != KIND {
        return Err(CoreError::invalid_input("该记录不是知识库采集会话"));
    }
    serde_json::from_value(record.session.state["knowledge_collection"].clone())
        .map_err(|e| CoreError::invalid_input(format!("采集记录元数据损坏: {e}")))
}

pub fn load(store: &SessionStore, kb_id: &str, id: &str) -> Result<SessionRecord> {
    let record = store.get(id)?;
    if metadata(&record)?.kb_id != kb_id {
        return Err(CoreError::invalid_input("采集记录不属于当前知识库"));
    }
    Ok(record)
}

pub fn prepare(
    store: &SessionStore,
    kb_id: &str,
    topic: &str,
    session_id: Option<&str>,
) -> Result<(Session, CollectionHistory)> {
    if let Some(id) = session_id {
        let record = load(store, kb_id, id)?;
        let history = metadata(&record)?;
        if history.topic != topic {
            return Err(CoreError::invalid_input(
                "继续采集不能更改原主题，请新建采集",
            ));
        }
        let mut session = record.session;
        session.messages.retain(|message| {
            !(message.is_system() && message.content.starts_with("[completion check] "))
        });
        Ok((session, history))
    } else {
        let mut session = Session::new(format!("采集 · {topic}"));
        session.push(Message::system("你是知识库采集 Agent，只能使用 web_fetch 与 knowledge_save 完成采集。不写手稿，不调用 write_file。"));
        Ok((
            session,
            CollectionHistory {
                kb_id: kb_id.into(),
                topic: topic.into(),
                runs: vec![],
            },
        ))
    }
}

pub fn save(
    store: &SessionStore,
    session: &mut Session,
    history: &CollectionHistory,
) -> Result<()> {
    session.state["knowledge_collection"] = serde_json::to_value(history)?;
    session.touch();
    store.save(&SessionRecord {
        session: session.clone(),
        kind: KIND.into(),
        goal: Some(history.topic.clone()),
    })
}

pub fn summary(record: &SessionRecord) -> Result<serde_json::Value> {
    let history = metadata(record)?;
    Ok(serde_json::json!({
        "id": record.session.id, "topic": history.topic, "kb_id": history.kb_id,
        "created_ms": record.session.created_ms, "updated_ms": record.session.updated_ms,
        "runs": history.runs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_roundtrip_resume_and_scope_guards() {
        let root = std::env::temp_dir().join(na_common::next_id("collection_history"));
        let store = SessionStore::open(&root).unwrap();
        let (mut session, mut history) = prepare(&store, "kb-a", "测试题材", None).unwrap();
        session.push(Message::assistant("已保存两条资料，还需要地点资料"));
        history.runs.push(CollectionRun {
            started_ms: 1,
            finished_ms: Some(2),
            status: "partial".into(),
            added: 2,
            sources: 1,
            steps: 5,
            stopped_reason: "budget".into(),
            error: None,
            follow_up: String::new(),
        });
        save(&store, &mut session, &history).unwrap();
        let reopened = SessionStore::open(&root).unwrap();
        let (resumed, meta) =
            prepare(&reopened, "kb-a", "测试题材", Some(session.id.as_str())).unwrap();
        assert_eq!(resumed, session);
        assert_eq!(meta.runs[0].added, 2);
        assert_eq!(reopened.list().unwrap().len(), 1);
        assert!(prepare(&reopened, "kb-b", "测试题材", Some(session.id.as_str())).is_err());
        assert!(prepare(&reopened, "kb-a", "另一个主题", Some(session.id.as_str())).is_err());
        assert!(load(&reopened, "kb-a", "../bad").is_err());
        let other_work = SessionStore::open(root.join("other-work")).unwrap();
        assert!(load(&other_work, "kb-a", session.id.as_str()).is_err());
        let mut continued = resumed;
        continued.push(Message::user("继续查找地点资料"));
        save(&reopened, &mut continued, &meta).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 1);
        assert_eq!(
            load(&reopened, "kb-a", session.id.as_str())
                .unwrap()
                .session
                .messages
                .len(),
            session.messages.len() + 1
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
