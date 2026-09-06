use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use na_library::WorkMeta;
use serde::{Deserialize, Serialize};

const LEGACY_DATA_DIR_NAMES: [&str; 2] = [
    "com.novelgenerateteam.desktop",
    "novel-generate-team-desktop",
];
const INDEX_NAME: &str = "works_index.json";
const INDEX_BACKUP_NAME: &str = ".works_index.before-app-id-migration.bak";
const MIGRATION_MARKER_HEADER: &str = "NOVEL_GENERATE_AGENT_DATA_MIGRATION_V1\n";
const OWNED_ROOT_ENTRIES: [&str; 5] = [
    "providers.json",
    "works",
    "workspace",
    "sessions",
    "knowledge",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WorkIndex {
    #[serde(default)]
    works: Vec<WorkMeta>,
    #[serde(default)]
    active: Option<String>,
}

/// Import data from the pre-rename Tauri application directory into the
/// current app-data root. The legacy root is never modified or deleted.
pub(crate) fn migrate_known_legacy_data(current_root: &Path) -> Result<Option<PathBuf>, String> {
    fs::create_dir_all(current_root)
        .map_err(|error| format!("无法创建新版数据目录 {}: {error}", current_root.display()))?;
    recover_interrupted_index_write(current_root)?;

    let parent = current_root
        .parent()
        .ok_or_else(|| format!("新版数据目录缺少父目录: {}", current_root.display()))?;
    for name in LEGACY_DATA_DIR_NAMES {
        let legacy_root = parent.join(name);
        if !has_owned_data(&legacy_root) || paths_resolve_equal(current_root, &legacy_root) {
            continue;
        }
        migrate_legacy_root(current_root, &legacy_root, name)?;
        return Ok(Some(legacy_root));
    }
    Ok(None)
}

fn migrate_legacy_root(
    current_root: &Path,
    legacy_root: &Path,
    legacy_name: &str,
) -> Result<(), String> {
    let marker = migration_marker(current_root, legacy_name);
    if fs::read_to_string(&marker)
        .map(|content| content.starts_with(MIGRATION_MARKER_HEADER))
        .unwrap_or(false)
    {
        return Ok(());
    }

    let legacy_root = fs::canonicalize(legacy_root)
        .map_err(|error| format!("无法定位旧版数据目录 {}: {error}", legacy_root.display()))?;
    let current_root = fs::canonicalize(current_root)
        .map_err(|error| format!("无法定位新版数据目录 {}: {error}", current_root.display()))?;
    if paths_resolve_equal(&current_root, &legacy_root) {
        return Ok(());
    }

    let legacy_index = read_index(&legacy_root.join(INDEX_NAME))?;
    let mut current_index = read_index(&current_root.join(INDEX_NAME))?.unwrap_or_default();

    let conflict_root = current_root
        .join(".legacy-data-conflicts")
        .join(legacy_name);
    for entry_name in OWNED_ROOT_ENTRIES {
        let source = legacy_root.join(entry_name);
        if !source.exists() {
            continue;
        }
        copy_entry_merge(
            &source,
            &current_root.join(entry_name),
            &conflict_root.join(entry_name),
        )?;
    }

    let mut imported_ids = Vec::new();
    if let Some(index) = legacy_index {
        merge_index(
            &mut current_index,
            index,
            &legacy_root,
            &current_root,
            &mut imported_ids,
        )?;
    }

    if imported_ids.is_empty() && legacy_root.join("workspace").is_dir() {
        if let Some(id) = add_unindexed_legacy_workspace(&mut current_index, &current_root)? {
            imported_ids.push(id);
        }
    }

    if !imported_ids.is_empty() || legacy_root.join(INDEX_NAME).is_file() {
        write_index_atomically(&current_root.join(INDEX_NAME), &current_index)?;
    }

    let marker_content = format!(
        "{MIGRATION_MARKER_HEADER}source={}\n",
        legacy_root.display()
    );
    fs::write(&marker, marker_content.as_bytes()).map_err(|error| {
        format!(
            "旧版数据已经复制，但无法写入迁移标记 {}: {error}",
            marker.display()
        )
    })?;
    Ok(())
}

fn has_owned_data(root: &Path) -> bool {
    root.is_dir()
        && (root.join(INDEX_NAME).is_file()
            || OWNED_ROOT_ENTRIES
                .iter()
                .any(|entry| root.join(entry).exists()))
}

fn migration_marker(current_root: &Path, legacy_name: &str) -> PathBuf {
    current_root.join(format!(".migrated-{legacy_name}-v1"))
}

fn read_index(path: &Path) -> Result<Option<WorkIndex>, String> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(format!("作品索引不是普通文件: {}", path.display()));
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("无法读取作品索引 {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("无法解析作品索引 {}: {error}", path.display()))
}

fn merge_index(
    current: &mut WorkIndex,
    legacy: WorkIndex,
    legacy_root: &Path,
    current_root: &Path,
    imported_ids: &mut Vec<String>,
) -> Result<(), String> {
    let mut ids: HashSet<String> = current.works.iter().map(|work| work.id.clone()).collect();
    let legacy_active = legacy.active;

    for legacy_work in legacy.works {
        let work = rebase_work(legacy_work, legacy_root, current_root)?;
        if let Some(existing) = current.works.iter().find(|item| item.id == work.id) {
            if existing == &work {
                imported_ids.push(work.id.clone());
                continue;
            }
            return Err(format!(
                "新旧数据中存在不同内容的同名作品标识 {}。旧数据仍保留在 {}",
                work.id,
                legacy_root.display()
            ));
        }
        if !ids.insert(work.id.clone()) {
            return Err(format!("旧版作品索引包含重复标识: {}", work.id));
        }
        imported_ids.push(work.id.clone());
        current.works.push(work);
    }

    if let Some(active) = legacy_active {
        if ids.contains(&active) {
            current.active = Some(active);
        }
    } else if let Some(first) = imported_ids.first() {
        current.active = Some(first.clone());
    }
    Ok(())
}

fn rebase_work(
    mut work: WorkMeta,
    legacy_root: &Path,
    current_root: &Path,
) -> Result<WorkMeta, String> {
    work.workspace_dir = rebase_path(&work.workspace_dir, legacy_root, current_root)?;
    work.sessions_dir = rebase_path(&work.sessions_dir, legacy_root, current_root)?;
    work.knowledge_dir = rebase_path(&work.knowledge_dir, legacy_root, current_root)?;
    Ok(work)
}

fn rebase_path(path: &Path, legacy_root: &Path, current_root: &Path) -> Result<PathBuf, String> {
    if let Ok(relative) = path.strip_prefix(legacy_root) {
        return Ok(current_root.join(relative));
    }
    if let Some(relative) = textual_relative_path(path, legacy_root) {
        return Ok(current_root.join(relative));
    }

    let canonical_path = fs::canonicalize(path).map_err(|error| {
        format!(
            "旧版作品路径无法访问，拒绝猜测迁移位置 {}: {error}",
            path.display()
        )
    })?;
    let canonical_root = fs::canonicalize(legacy_root)
        .map_err(|error| format!("旧版数据根目录无法访问 {}: {error}", legacy_root.display()))?;
    let relative = canonical_path
        .strip_prefix(&canonical_root)
        .map_err(|_| format!("旧版作品路径越出旧数据目录，拒绝迁移: {}", path.display()))?;
    Ok(current_root.join(relative))
}

fn add_unindexed_legacy_workspace(
    index: &mut WorkIndex,
    current_root: &Path,
) -> Result<Option<String>, String> {
    let workspace = current_root.join("workspace");
    if index
        .works
        .iter()
        .any(|work| paths_resolve_equal(&work.workspace_dir, &workspace))
    {
        return Ok(None);
    }

    let id = na_common::next_id("work");
    let base = current_root.join("works").join(&id);
    let sessions = current_root.join("sessions");
    let knowledge = base.join("knowledge");
    fs::create_dir_all(&sessions)
        .map_err(|error| format!("无法创建迁移会话目录 {}: {error}", sessions.display()))?;
    fs::create_dir_all(&knowledge)
        .map_err(|error| format!("无法创建迁移知识库目录 {}: {error}", knowledge.display()))?;
    let now = now_millis();
    index.works.push(WorkMeta {
        id: id.clone(),
        title: "默认作品".to_string(),
        blurb: "从旧版应用数据自动恢复的原有手稿".to_string(),
        genre: String::new(),
        source_material: String::new(),
        created_ms: now,
        updated_ms: now,
        workspace_dir: workspace,
        sessions_dir: sessions,
        knowledge_dir: knowledge,
    });
    index.active = Some(id.clone());
    Ok(Some(id))
}

fn copy_entry_merge(source: &Path, destination: &Path, conflict: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("无法检查旧版数据 {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "旧版数据包含文件系统链接，已停止迁移以避免越界: {}",
            source.display()
        ));
    }
    if fs::symlink_metadata(destination)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return copy_to_conflict(source, conflict);
    }

    if metadata.is_dir() {
        if destination.exists() && !destination.is_dir() {
            return copy_to_conflict(source, conflict);
        }
        fs::create_dir_all(destination)
            .map_err(|error| format!("无法创建迁移目录 {}: {error}", destination.display()))?;
        for entry in fs::read_dir(source)
            .map_err(|error| format!("无法读取旧版目录 {}: {error}", source.display()))?
        {
            let entry = entry
                .map_err(|error| format!("无法读取旧版目录项 {}: {error}", source.display()))?;
            let name = entry.file_name();
            copy_entry_merge(
                &entry.path(),
                &destination.join(&name),
                &conflict.join(&name),
            )?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        return Err(format!("旧版数据不是普通文件: {}", source.display()));
    }
    if destination.exists() {
        if destination.is_file() && files_equal(source, destination)? {
            return Ok(());
        }
        return copy_to_conflict(source, conflict);
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建迁移文件父目录 {}: {error}", parent.display()))?;
    }
    fs::copy(source, destination).map_err(|error| {
        format!(
            "无法复制旧版数据 {} 到 {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn copy_to_conflict(source: &Path, conflict: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("无法检查冲突数据 {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "旧版冲突数据包含文件系统链接: {}",
            source.display()
        ));
    }
    if !metadata.is_dir() && !metadata.is_file() {
        return Err(format!("冲突数据不是普通文件或目录: {}", source.display()));
    }
    let conflict_metadata = fs::symlink_metadata(conflict).ok();
    if metadata.is_file()
        && conflict_metadata
            .as_ref()
            .is_some_and(|item| item.is_file())
        && files_equal(source, conflict)?
    {
        return Ok(());
    }
    let target = if conflict_metadata.is_some() {
        unique_conflict_path(conflict)
    } else {
        conflict.to_path_buf()
    };
    if metadata.is_dir() {
        fs::create_dir_all(&target)
            .map_err(|error| format!("无法创建冲突备份目录 {}: {error}", target.display()))?;
        for entry in fs::read_dir(source)
            .map_err(|error| format!("无法读取冲突目录 {}: {error}", source.display()))?
        {
            let entry = entry
                .map_err(|error| format!("无法读取冲突目录项 {}: {error}", source.display()))?;
            let name = entry.file_name();
            copy_entry_merge(&entry.path(), &target.join(&name), &target.join(&name))?;
        }
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建冲突备份目录 {}: {error}", parent.display()))?;
    }
    fs::copy(source, &target).map_err(|error| {
        format!(
            "无法保存冲突旧数据 {} 到 {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn unique_conflict_path(preferred: &Path) -> PathBuf {
    if fs::symlink_metadata(preferred).is_err() {
        return preferred.to_path_buf();
    }
    let file_name = preferred
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "legacy-data".to_string());
    for suffix in 1..10_000_u32 {
        let candidate = preferred.with_file_name(format!("{file_name}.{suffix}.legacy"));
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    preferred.with_file_name(format!("{file_name}.{}.legacy", now_millis()))
}

fn files_equal(left: &Path, right: &Path) -> Result<bool, String> {
    let left_len = fs::metadata(left)
        .map_err(|error| format!("无法检查文件 {}: {error}", left.display()))?
        .len();
    let right_len = fs::metadata(right)
        .map_err(|error| format!("无法检查文件 {}: {error}", right.display()))?
        .len();
    if left_len != right_len {
        return Ok(false);
    }

    let mut left_file = fs::File::open(left)
        .map_err(|error| format!("无法读取文件 {}: {error}", left.display()))?;
    let mut right_file = fs::File::open(right)
        .map_err(|error| format!("无法读取文件 {}: {error}", right.display()))?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_read = left_file
            .read(&mut left_buffer)
            .map_err(|error| format!("无法读取文件 {}: {error}", left.display()))?;
        let right_read = right_file
            .read(&mut right_buffer)
            .map_err(|error| format!("无法读取文件 {}: {error}", right.display()))?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

fn write_index_atomically(path: &Path, index: &WorkIndex) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("作品索引缺少父目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建作品索引目录 {}: {error}", parent.display()))?;
    recover_interrupted_index_write(parent)?;

    let temporary = parent.join(format!(
        ".{INDEX_NAME}.{}.migration.tmp",
        std::process::id()
    ));
    let backup = parent.join(INDEX_BACKUP_NAME);
    let _ = fs::remove_file(&temporary);
    let json = serde_json::to_vec_pretty(index)
        .map_err(|error| format!("无法序列化迁移后的作品索引: {error}"))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("无法创建临时作品索引 {}: {error}", temporary.display()))?;
    file.write_all(&json)
        .map_err(|error| format!("无法写入临时作品索引 {}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("无法刷新临时作品索引 {}: {error}", temporary.display()))?;
    drop(file);
    read_index(&temporary)?.ok_or_else(|| "临时作品索引写入后消失".to_string())?;

    if path.exists() {
        fs::rename(path, &backup).map_err(|error| {
            format!(
                "无法备份现有作品索引 {} 到 {}: {error}",
                path.display(),
                backup.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "无法提交迁移后的作品索引 {}: {error}",
            path.display()
        ));
    }
    match read_index(path) {
        Ok(Some(written)) if written == *index => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        result => {
            let _ = fs::remove_file(path);
            if backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            Err(format!("迁移后的作品索引校验失败: {result:?}"))
        }
    }
}

fn recover_interrupted_index_write(root: &Path) -> Result<(), String> {
    let index = root.join(INDEX_NAME);
    let backup = root.join(INDEX_BACKUP_NAME);
    if !backup.exists() {
        return Ok(());
    }
    if read_index(&index).is_ok_and(|value| value.is_some()) {
        fs::remove_file(&backup)
            .map_err(|error| format!("无法清理旧作品索引备份 {}: {error}", backup.display()))?;
        return Ok(());
    }
    if index.exists() {
        fs::remove_file(&index)
            .map_err(|error| format!("无法移除损坏作品索引 {}: {error}", index.display()))?;
    }
    fs::rename(&backup, &index).map_err(|error| {
        format!(
            "无法恢复迁移前作品索引 {} 到 {}: {error}",
            backup.display(),
            index.display()
        )
    })?;
    Ok(())
}

fn paths_resolve_equal(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => normalized_path(&left) == normalized_path(&right),
        _ => normalized_path(left) == normalized_path(right),
    }
}

fn textual_relative_path(path: &Path, root: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let path = normalized_path(path);
        let root = normalized_path(root);
        if path == root {
            return Some(PathBuf::new());
        }
        let prefix = format!("{root}\\");
        path.strip_prefix(&prefix).map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        path.strip_prefix(root).ok().map(PathBuf::from)
    }
}

#[cfg(windows)]
fn normalized_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('/', "\\");
    let ordinary = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw
    };
    ordinary.trim_end_matches('\\').to_lowercase()
}

#[cfg(not(windows))]
fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches('/').to_string()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_library::WorkStore;

    fn temp_parent(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "desktop_data_migration_{tag}_{}",
            na_common::next_id("test")
        ))
    }

    fn write_index(root: &Path, index: &WorkIndex) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join(INDEX_NAME),
            serde_json::to_vec_pretty(index).unwrap(),
        )
        .unwrap();
    }

    fn legacy_work(root: &Path, id: &str) -> WorkMeta {
        WorkMeta {
            id: id.to_string(),
            title: "旧作".to_string(),
            blurb: "原有数据".to_string(),
            genre: "玄幻".to_string(),
            source_material: String::new(),
            created_ms: 100,
            updated_ms: 200,
            workspace_dir: root.join("workspace"),
            sessions_dir: root.join("sessions"),
            knowledge_dir: root.join("works").join(id).join("knowledge"),
        }
    }

    #[test]
    fn renamed_app_data_is_copied_rebased_and_left_as_backup() {
        let parent = temp_parent("renamed");
        let current = parent.join("com.novelgenerateagent.desktop");
        let legacy = parent.join("com.novelgenerateteam.desktop");
        let work = legacy_work(&legacy, "work_legacy_1");
        fs::create_dir_all(work.workspace_dir.join("book")).unwrap();
        fs::create_dir_all(&work.sessions_dir).unwrap();
        fs::write(work.workspace_dir.join("book/chapter.md"), "旧章节").unwrap();
        fs::write(work.sessions_dir.join("sess_old.json"), "{}").unwrap();
        fs::write(legacy.join("providers.json"), "{\"providers\":[]}").unwrap();
        write_index(
            &legacy,
            &WorkIndex {
                works: vec![work],
                active: Some("work_legacy_1".to_string()),
            },
        );
        fs::create_dir_all(&current).unwrap();
        fs::write(
            migration_marker(&current, "com.novelgenerateteam.desktop"),
            "truncated marker",
        )
        .unwrap();

        let migrated = migrate_known_legacy_data(&current).unwrap();
        assert_eq!(migrated.as_deref(), Some(legacy.as_path()));
        assert_eq!(
            fs::read_to_string(current.join("workspace/book/chapter.md")).unwrap(),
            "旧章节"
        );
        assert!(legacy.join("workspace/book/chapter.md").is_file());
        assert!(current.join("providers.json").is_file());

        let store = WorkStore::open(&current).unwrap();
        let active = store.active().unwrap();
        assert_eq!(active.id, "work_legacy_1");
        assert!(active
            .workspace_dir
            .starts_with(fs::canonicalize(&current).unwrap()));
        assert!(!active
            .workspace_dir
            .starts_with(fs::canonicalize(&legacy).unwrap()));

        fs::write(legacy.join("workspace/book/chapter.md"), "旧版后来变化").unwrap();
        migrate_known_legacy_data(&current).unwrap();
        assert_eq!(
            fs::read_to_string(current.join("workspace/book/chapter.md")).unwrap(),
            "旧章节"
        );
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn legacy_library_merges_with_a_new_starter_work() {
        let parent = temp_parent("merge");
        let current = parent.join("com.novelgenerateagent.desktop");
        let legacy = parent.join("com.novelgenerateteam.desktop");

        let mut current_store = WorkStore::open(&current).unwrap();
        let starter = current_store.create("我的第一部作品", "", "", "").unwrap();
        drop(current_store);

        let work = legacy_work(&legacy, "work_legacy_2");
        fs::create_dir_all(work.workspace_dir.join("book")).unwrap();
        fs::create_dir_all(&work.sessions_dir).unwrap();
        fs::create_dir_all(&work.knowledge_dir).unwrap();
        fs::write(work.workspace_dir.join("book/old.md"), "保留").unwrap();
        write_index(
            &legacy,
            &WorkIndex {
                works: vec![work],
                active: Some("work_legacy_2".to_string()),
            },
        );

        migrate_known_legacy_data(&current).unwrap();
        let merged = WorkStore::open(&current).unwrap();
        assert_eq!(merged.list().len(), 2);
        assert!(merged.get(&starter.id).is_some());
        assert_eq!(merged.active_id(), Some("work_legacy_2"));
        assert!(merged
            .active()
            .unwrap()
            .workspace_dir
            .join("book/old.md")
            .is_file());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn unindexed_workspace_is_restored_even_after_new_app_created_a_work() {
        let parent = temp_parent("unindexed");
        let current = parent.join("com.novelgenerateagent.desktop");
        let legacy = parent.join("novel-generate-team-desktop");
        let mut current_store = WorkStore::open(&current).unwrap();
        current_store.create("新作品", "", "", "").unwrap();
        drop(current_store);
        fs::create_dir_all(legacy.join("workspace/book")).unwrap();
        fs::write(legacy.join("workspace/book/legacy.md"), "仍然存在").unwrap();

        migrate_known_legacy_data(&current).unwrap();
        let merged = WorkStore::open(&current).unwrap();
        assert_eq!(merged.list().len(), 2);
        assert_eq!(
            fs::read_to_string(
                merged
                    .active()
                    .unwrap()
                    .workspace_dir
                    .join("book/legacy.md")
            )
            .unwrap(),
            "仍然存在"
        );
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn conflicting_files_keep_current_and_archive_legacy_copy() {
        let parent = temp_parent("conflict");
        let current = parent.join("com.novelgenerateagent.desktop");
        let legacy = parent.join("com.novelgenerateteam.desktop");
        fs::create_dir_all(current.join("workspace/book")).unwrap();
        fs::create_dir_all(legacy.join("workspace/book")).unwrap();
        fs::write(current.join("workspace/book/chapter.md"), "new").unwrap();
        fs::write(legacy.join("workspace/book/chapter.md"), "old").unwrap();

        migrate_known_legacy_data(&current).unwrap();
        assert_eq!(
            fs::read_to_string(current.join("workspace/book/chapter.md")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(current.join(
                ".legacy-data-conflicts/com.novelgenerateteam.desktop/workspace/book/chapter.md"
            ))
            .unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(legacy.join("workspace/book/chapter.md")).unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn cache_only_directory_is_not_treated_as_user_data() {
        let parent = temp_parent("cache_only");
        let current = parent.join("com.novelgenerateagent.desktop");
        let legacy = parent.join("novel-generate-team-desktop");
        fs::create_dir_all(legacy.join("Cache")).unwrap();
        fs::write(legacy.join("Cache/blob"), "cache").unwrap();

        assert!(migrate_known_legacy_data(&current).unwrap().is_none());
        assert!(!current.join("Cache").exists());
        let _ = fs::remove_dir_all(parent);
    }
}
