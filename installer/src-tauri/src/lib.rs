//! Custom frameless installer backend.
//!
//! The whole main application is a single self-contained exe, embedded here at
//! compile time via `include_bytes!`, so the installer ships as ONE file. The
//! Rust side detects an existing install (registry + default path), copies the
//! payload to the chosen directory (updating in place / stopping a running
//! instance), creates Start-Menu + Desktop shortcuts, and writes the uninstall
//! registry entry — all per-user (under %LOCALAPPDATA%), so no UAC is required.
//!
//! Shortcuts are created **in-process** through the Windows Shell COM API
//! (`IShellLinkW` + `IPersistFile`), with the target folders resolved via
//! `SHGetKnownFolderPath`. This is locale-proof (no command-line encoding of
//! CJK paths), independent of PowerShell / execution policy, and — crucially —
//! honors OneDrive "known folder" redirection so the Desktop shortcut lands on
//! the *real* desktop. A PowerShell path is kept only as a last-ditch fallback.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;
use tauri::Emitter;

/// Suppress the console window that spawning a CLI helper (reg/powershell/taskkill)
/// would otherwise flash from this GUI app. Without this the install flickered a
/// flurry of black command windows.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Build a `Command` that never pops a console window.
fn hidden(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Run a helper command silently (no window), ignoring its output.
fn run_hidden(program: &str, args: &[&str]) -> bool {
    hidden(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run a helper command silently and capture its output.
fn output_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    hidden(program).args(args).output()
}

/// Shell and CIM APIs expect ordinary DOS/UNC paths rather than Win32's
/// extended-length spelling returned by `canonicalize`.
fn win32_compatible_path(path: &Path) -> PathBuf {
    const EXTENDED_UNC_PREFIX: &str = r"\\?\UNC\";
    const EXTENDED_PREFIX: &str = r"\\?\";

    let text = path.to_string_lossy();
    if text
        .get(..EXTENDED_UNC_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(EXTENDED_UNC_PREFIX))
    {
        return PathBuf::from(format!(r"\\{}", &text[EXTENDED_UNC_PREFIX.len()..]));
    }

    if text
        .get(..EXTENDED_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(EXTENDED_PREFIX))
    {
        let rest = &text[EXTENDED_PREFIX.len()..];
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
        {
            return PathBuf::from(rest);
        }
    }

    path.to_path_buf()
}

const POWERSHELL_WIN32_PATH_FUNCTION: &str =
    "function Convert-NgtWin32Path([string]$Path) {\r\n\
     \x20 $Full = [IO.Path]::GetFullPath($Path)\r\n\
     \x20 if ($Full.StartsWith('\\\\?\\UNC\\', [StringComparison]::OrdinalIgnoreCase)) { return '\\\\' + $Full.Substring(8) }\r\n\
     \x20 if ($Full.StartsWith('\\\\?\\', [StringComparison]::OrdinalIgnoreCase) -and $Full.Length -ge 7 -and $Full[4] -match '[A-Za-z]' -and $Full[5] -eq ':' -and ($Full[6] -eq '\\' -or $Full[6] -eq '/')) { return $Full.Substring(4) }\r\n\
     \x20 return $Full\r\n\
     }\r\n";

fn process_query_script(executable: &Path) -> String {
    let executable = win32_compatible_path(executable);
    let process_name = executable
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| EXE_NAME.to_string());
    format!(
        "$ErrorActionPreference='Stop'; {path_normalizer}$target=Convert-NgtWin32Path '{target}'; \
         Get-CimInstance Win32_Process -Filter \"Name='{name}'\" | \
         Where-Object {{ $_.ExecutablePath -and \
         [StringComparer]::OrdinalIgnoreCase.Equals((Convert-NgtWin32Path $_.ExecutablePath),$target) }} | \
         ForEach-Object {{ $_.ProcessId }}",
        path_normalizer = POWERSHELL_WIN32_PATH_FUNCTION,
        target = ps_quote(&executable.to_string_lossy()),
        name = ps_quote(&process_name),
    )
}

fn parse_process_ids(stdout: &[u8]) -> Result<Vec<u32>, String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.parse::<u32>()
                .map_err(|_| format!("进程查询返回了无效 PID: {line}"))
        })
        .collect()
}

fn matching_process_ids(executable: &Path) -> Result<Vec<u32>, String> {
    let script = process_query_script(executable);
    let output = output_hidden(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .map_err(|error| format!("无法启动进程查询: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("进程查询失败，退出码: {}", output.status)
        } else {
            format!("进程查询失败: {detail}")
        });
    }
    parse_process_ids(&output.stdout)
}

fn stop_processes_for_executable(executable: &Path) -> Result<usize, String> {
    let process_ids = matching_process_ids(executable)?;
    for process_id in &process_ids {
        let process_id = process_id.to_string();
        let _ = run_hidden("taskkill", &["/PID", &process_id]);
    }

    if !process_ids.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(400));
        for process_id in matching_process_ids(executable)? {
            let process_id = process_id.to_string();
            let _ = run_hidden("taskkill", &["/F", "/PID", &process_id]);
        }
    }
    Ok(process_ids.len())
}

fn stop_and_confirm_processes(executable: &Path) -> Result<(), String> {
    let _ = stop_processes_for_executable(executable)?;
    for _ in 0..10 {
        if matching_process_ids(executable)?.is_empty() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err("已安装程序在限定时间内仍未退出。".into())
}

/// The entire main app (a single self-contained Tauri exe), embedded.
const PAYLOAD: &[u8] = include_bytes!(env!("NGT_PAYLOAD_PATH"));
const PAYLOAD_KIND: &str = env!("NGT_PAYLOAD_IS_STUB");

const EXE_NAME: &str = "NovelGenerateAgent.exe";
// The product was renamed from NovelGenerateTeam before the current installer
// was published. Keep these legacy names only for a verified in-place upgrade.
const LEGACY_EXE_NAME: &str = "NovelGenerateTeam.exe";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const PUBLISHER: &str = "Novel Generate Agent";
const DISPLAY_NAME: &str = "Novel Generate Agent (墨·创作)";
const SHORTCUT_NAME: &str = "墨·创作.lnk";
const INSTALL_MARKER_NAME: &str = ".novel-generate-agent-install";
const INSTALL_MARKER_CONTENT: &str = "NOVEL_GENERATE_AGENT_INSTALL_V1\n";
const INSTALLER_CACHE_IDENTIFIERS: [&str; 2] = [
    "com.novelgenerateagent.installer",
    "com.novelgenerateteam.installer",
];
const DESKTOP_CACHE_IDENTIFIERS: [&str; 2] = [
    "com.novelgenerateagent.desktop",
    "com.novelgenerateteam.desktop",
];
const UNINSTALL_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateAgent";
const UNINSTALL_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateAgent";
const LEGACY_UNINSTALL_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateTeam";
const LEGACY_UNINSTALL_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateTeam";

#[derive(Serialize, Clone)]
struct DetectResult {
    installed: bool,
    path: String,
    version: Option<String>,
}

/// Returned to the UI when an install finishes, so the Done screen can warn the
/// user if shortcuts could not be created (instead of silently claiming success).
#[derive(Serialize, Clone)]
struct InstallReport {
    /// Number of shortcuts successfully created.
    shortcuts: usize,
    /// Human-readable per-target failures (empty on full success).
    shortcut_errors: Vec<String>,
    /// Whether the uninstaller was written + registered.
    uninstaller: bool,
    /// Diagnostic detail when the GUI uninstaller was not made ready.
    uninstaller_error: Option<String>,
}

fn local_appdata() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

fn default_install_dir() -> PathBuf {
    local_appdata().join("Programs").join("NovelGenerateAgent")
}

fn legacy_default_install_dir() -> PathBuf {
    local_appdata().join("Programs").join("NovelGenerateTeam")
}

fn same_path(a: &Path, b: &Path) -> bool {
    let normalize = |path: PathBuf| normalized_path_text(&path);
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => normalize(a) == normalize(b),
        _ => false,
    }
}

fn normalized_path_text(path: &Path) -> String {
    win32_compatible_path(path)
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
        || path
            .parent()
            .is_some_and(|parent| parent.as_os_str().is_empty())
}

fn is_protected_install_dir(path: &Path) -> bool {
    if !path.is_absolute() || is_filesystem_root(path) {
        return true;
    }

    ["USERPROFILE", "LOCALAPPDATA", "APPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain(std::iter::once(std::env::temp_dir()))
        .any(|protected| same_path(path, &protected))
}

fn normalize_install_target(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("安装目录必须是绝对路径。".into());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("安装目录不能包含 . 或 .. 路径片段。".into());
    }

    if path.exists() {
        return std::fs::canonicalize(path).map_err(|e| format!("无法规范化安装目录: {e}"));
    }

    let mut cursor = path.to_path_buf();
    let mut missing = Vec::new();
    while !cursor.exists() {
        let name = cursor
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "安装目录缺少有效目录名。".to_string())?;
        missing.push(name.to_os_string());
        cursor = cursor
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| "安装目录缺少可访问的父目录。".to_string())?
            .to_path_buf();
    }
    if !cursor.is_dir() {
        return Err("安装目录最近的现有父路径不是目录。".into());
    }

    let mut normalized =
        std::fs::canonicalize(&cursor).map_err(|e| format!("安装目录的父目录无法访问: {e}"))?;
    for component in missing.into_iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

fn has_install_marker(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(INSTALL_MARKER_NAME))
        .map(|content| content == INSTALL_MARKER_CONTENT)
        .unwrap_or(false)
}

fn has_uninstaller_companion(dir: &Path) -> bool {
    dir.join("uninstall.exe").is_file() || dir.join("uninstall.ps1").is_file()
}

fn has_legacy_install_layout(dir: &Path) -> bool {
    dir.join(LEGACY_EXE_NAME).is_file() && has_uninstaller_companion(dir)
}

fn has_unmarked_current_install_layout(dir: &Path) -> bool {
    dir.join(EXE_NAME).is_file() && has_uninstaller_companion(dir)
}

fn is_product_install_dir(dir: &Path) -> bool {
    (has_install_marker(dir) || has_unmarked_current_install_layout(dir))
        && dir.join(EXE_NAME).is_file()
}

fn registry_owns_install(dir: &Path) -> bool {
    dir.join(EXE_NAME).is_file() && registry_location_matches(UNINSTALL_KEY, dir)
}

fn registry_owns_legacy_install(dir: &Path) -> bool {
    dir.join(LEGACY_EXE_NAME).is_file() && registry_location_matches(LEGACY_UNINSTALL_KEY, dir)
}

fn registry_location_matches(key: &str, dir: &Path) -> bool {
    reg_read_from(key, "InstallLocation")
        .map(|registered| same_path(dir, Path::new(&registered)))
        .unwrap_or(false)
}

fn is_verified_install_dir(dir: &Path) -> bool {
    is_product_install_dir(dir)
        || registry_owns_install(dir)
        || has_legacy_install_layout(dir)
        || registry_owns_legacy_install(dir)
}

fn validate_install_target(dir: &Path) -> Result<PathBuf, String> {
    let dir = normalize_install_target(dir)?;
    if is_protected_install_dir(&dir) {
        return Err(
            "安装目录必须是安全的绝对应用子目录，不能使用磁盘根目录或用户数据根目录。".into(),
        );
    }
    if dir.exists() && !dir.is_dir() {
        return Err("安装路径已存在且不是目录。".into());
    }
    if dir.is_dir() {
        let non_empty = std::fs::read_dir(&dir)
            .map_err(|e| format!("无法检查安装目录: {e}"))?
            .next()
            .is_some();
        if non_empty && !is_verified_install_dir(&dir) {
            return Err(
                "所选目录不是空目录，也不是已验证的墨·创作安装目录。请选择空的专用目录，或选择旧版墨·创作安装目录。".into(),
            );
        }
    }
    Ok(dir)
}

fn validate_uninstall_target(dir: &Path) -> Result<PathBuf, String> {
    let dir = normalize_install_target(dir)?;
    if is_protected_install_dir(&dir) {
        return Err("拒绝卸载：目标是受保护目录。".into());
    }
    let marker = has_install_marker(&dir);
    let current_exe = dir.join(EXE_NAME).is_file();
    let legacy_exe = dir.join(LEGACY_EXE_NAME).is_file();
    let current_registry = registry_owns_install(&dir);
    let legacy_registry = registry_owns_legacy_install(&dir);
    let verified = is_product_install_dir(&dir)
        || current_registry
        || has_legacy_install_layout(&dir)
        || legacy_registry;
    if !marker && !current_exe && !legacy_exe && !current_registry && !legacy_registry {
        return Err("拒绝卸载：安装标记缺失或无效。请手动检查该目录。".into());
    }
    if !current_exe && !legacy_exe {
        return Err("拒绝卸载：主程序文件缺失。请手动检查该目录。".into());
    }
    if !verified {
        return Err("拒绝卸载：未找到受信任的墨·创作安装布局。请手动检查该目录。".into());
    }
    Ok(dir)
}

/// Read a REG_SZ value from an uninstall key, if present.
fn reg_read_from(key: &str, value: &str) -> Option<String> {
    let out = output_hidden("reg", &["query", key, "/v", value]).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(idx) = line.find("REG_SZ") {
            let rest = line[idx + "REG_SZ".len()..].trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

fn reg_read(value: &str) -> Option<String> {
    reg_read_from(UNINSTALL_KEY, value)
}

/// Detect whether the app is already installed, and where.
#[tauri::command]
fn detect_existing() -> DetectResult {
    // 1) Registry InstallLocation from the current installer.
    if let Some(loc) = reg_read("InstallLocation") {
        let path = Path::new(&loc);
        if path.join(EXE_NAME).is_file() || path.join(LEGACY_EXE_NAME).is_file() {
            return DetectResult {
                installed: true,
                path: loc,
                version: reg_read("DisplayVersion"),
            };
        }
    }

    // 2) The previous product name used a different executable and uninstall
    // key. Only accept that location when the old executable is still there.
    if let Some(loc) = reg_read_from(LEGACY_UNINSTALL_KEY, "InstallLocation") {
        let path = Path::new(&loc);
        if path.join(LEGACY_EXE_NAME).is_file() || path.join(EXE_NAME).is_file() {
            return DetectResult {
                installed: true,
                path: loc,
                version: reg_read_from(LEGACY_UNINSTALL_KEY, "DisplayVersion"),
            };
        }
    }

    // 3) The default per-user locations on disk. This fallback covers an old
    // install whose registry entry was removed or was never written.
    let def = default_install_dir();
    if is_product_install_dir(&def) || has_legacy_install_layout(&def) {
        return DetectResult {
            installed: true,
            path: def.to_string_lossy().into_owned(),
            version: reg_read("DisplayVersion")
                .or_else(|| reg_read_from(LEGACY_UNINSTALL_KEY, "DisplayVersion")),
        };
    }

    let legacy_def = legacy_default_install_dir();
    if is_product_install_dir(&legacy_def) || has_legacy_install_layout(&legacy_def) {
        return DetectResult {
            installed: true,
            path: legacy_def.to_string_lossy().into_owned(),
            version: reg_read_from(LEGACY_UNINSTALL_KEY, "DisplayVersion")
                .or_else(|| reg_read("DisplayVersion")),
        };
    }

    // Fresh install — propose the default directory.
    DetectResult {
        installed: false,
        path: def.to_string_lossy().into_owned(),
        version: None,
    }
}

/// The default install directory (for the "fresh install" case).
#[tauri::command]
fn default_dir() -> String {
    default_install_dir().to_string_lossy().into_owned()
}

/// The version this installer ships.
#[tauri::command]
fn installer_version() -> String {
    VERSION.to_string()
}

fn emit(app: &tauri::AppHandle, percent: u32, message: &str) {
    let _ = app.emit(
        "install-progress",
        serde_json::json!({ "percent": percent, "message": message }),
    );
}

fn write_file_transactionally_with<F>(
    target: &Path,
    content: &[u8],
    commit: F,
) -> Result<(), String>
where
    F: FnOnce(&Path, &Path, &Path) -> Result<(), String>,
{
    let parent = target
        .parent()
        .ok_or_else(|| "目标文件缺少父目录。".to_string())?;
    let target_name = target
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "目标文件缺少有效文件名。".to_string())?
        .to_string_lossy();
    let temporary = parent.join(format!(".{target_name}.{}.tmp", std::process::id()));
    let backup = parent.join(format!(".{target_name}.previous"));
    let _ = std::fs::remove_file(&temporary);

    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("创建临时程序文件失败: {e}"))?;
        file.write_all(content)
            .map_err(|e| format!("写入临时程序文件失败: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("刷新临时程序文件失败: {e}"))?;
        drop(file);

        let staged_len = std::fs::metadata(&temporary)
            .map_err(|e| format!("检查临时程序文件失败: {e}"))?
            .len();
        if staged_len != content.len() as u64 {
            return Err(format!(
                "临时程序文件大小不完整: {staged_len}/{}",
                content.len()
            ));
        }

        commit(&temporary, target, &backup)?;
        let installed_len = std::fs::metadata(target)
            .map_err(|e| format!("检查已安装程序失败: {e}"))?
            .len();
        let content_matches = installed_len == content.len() as u64
            && file_matches_content(target, content)
                .map_err(|e| format!("校验已安装程序失败: {e}"))?;
        if !content_matches {
            if backup.exists() {
                let _ = std::fs::remove_file(target);
                let _ = std::fs::rename(&backup, target);
            } else {
                let _ = std::fs::remove_file(target);
            }
            return Err(format!(
                "已安装程序内容校验失败: {installed_len}/{}",
                content.len()
            ));
        }
        let _ = std::fs::remove_file(&backup);
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn file_matches_content(path: &Path, expected: &[u8]) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut offset = 0usize;

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(offset == expected.len());
        }
        let end = offset.saturating_add(read);
        if end > expected.len() || buffer[..read] != expected[offset..end] {
            return Ok(false);
        }
        offset = end;
    }
}

#[cfg(windows)]
fn commit_staged_file(temporary: &Path, target: &Path, backup: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    if !target.exists() {
        return std::fs::rename(temporary, target).map_err(|e| format!("安装程序文件失败: {e}"));
    }

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let target_w = wide(target);
    let temporary_w = wide(temporary);
    let backup_w = wide(backup);
    let mut last_error = String::new();
    for attempt in 0..12 {
        if !target.exists() && backup.exists() {
            let _ = std::fs::rename(backup, target);
        }
        if target.exists() && backup.exists() {
            let _ = std::fs::remove_file(backup);
        }

        // SAFETY: all pointers are NUL-terminated buffers kept alive for the
        // call; replacement and backup are sibling files on the same volume.
        let replaced = unsafe {
            ReplaceFileW(
                PCWSTR(target_w.as_ptr()),
                PCWSTR(temporary_w.as_ptr()),
                PCWSTR(backup_w.as_ptr()),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        };
        match replaced {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = error.to_string();
                if attempt < 11 && temporary.exists() {
                    std::thread::sleep(std::time::Duration::from_millis(125));
                }
            }
        }
    }

    if !target.exists() && backup.exists() {
        let _ = std::fs::rename(backup, target);
    }
    Err(format!("原子替换程序文件失败: {last_error}"))
}

#[cfg(not(windows))]
fn commit_staged_file(temporary: &Path, target: &Path, backup: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(backup);
    if target.exists() {
        std::fs::rename(target, backup).map_err(|e| format!("备份旧程序失败: {e}"))?;
    }
    if let Err(error) = std::fs::rename(temporary, target) {
        if backup.exists() {
            let _ = std::fs::rename(backup, target);
        }
        return Err(format!("替换程序文件失败: {error}"));
    }
    Ok(())
}

fn write_payload_transactionally(target: &Path) -> Result<(), String> {
    write_file_transactionally_with(target, PAYLOAD, commit_staged_file)
}

fn write_current_executable_transactionally(target: &Path) -> Result<(), String> {
    let source = std::env::current_exe().map_err(|e| format!("无法定位当前安装器: {e}"))?;
    let content = std::fs::read(&source).map_err(|e| format!("读取当前安装器失败: {e}"))?;
    write_file_transactionally_with(target, &content, commit_staged_file)
}

/// Perform the install / update into `dir`. Emits `install-progress` events.
///
/// Async + `spawn_blocking`: the heavy work (process spawns, 12MB write) runs off
/// the main thread so the window stays responsive and progress events keep flowing.
#[tauri::command]
async fn install(app: tauri::AppHandle, dir: String) -> Result<InstallReport, String> {
    tokio::task::spawn_blocking(move || install_blocking(&app, &dir))
        .await
        .map_err(|e| format!("安装任务异常: {e}"))?
}

fn install_blocking(app: &tauri::AppHandle, dir: &str) -> Result<InstallReport, String> {
    if PAYLOAD_KIND == "1" {
        return Err("安装器未嵌入主程序 payload，无法执行安装。请先完成发布构建步骤。".into());
    }
    let requested_dir = PathBuf::from(dir);
    let mut target_dir = validate_install_target(&requested_dir)?;
    let target_exe = target_dir.join(EXE_NAME);
    let legacy_exe = target_dir.join(LEGACY_EXE_NAME);
    let updating = target_exe.is_file() || legacy_exe.is_file();

    emit(
        app,
        6,
        if updating {
            "准备更新…"
        } else {
            "准备安装…"
        },
    );
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let created_dir =
        std::fs::canonicalize(&target_dir).map_err(|e| format!("无法确认实际安装目录: {e}"))?;
    if normalized_path_text(&target_dir) != normalized_path_text(&created_dir)
        || is_protected_install_dir(&created_dir)
    {
        return Err("安装目录在创建后发生重定向，已为安全起见中止安装。".into());
    }
    target_dir = created_dir;
    let target_exe = target_dir.join(EXE_NAME);
    let legacy_exe = target_dir.join(LEGACY_EXE_NAME);

    // Stop a running instance so the exe can be replaced (update-in-place).
    // During the rename migration both executable names may be present, so
    // each exact path is checked independently.
    if updating {
        emit(app, 22, "结束正在运行的旧版本…");
        for executable in [&target_exe, &legacy_exe] {
            if !executable.is_file() {
                continue;
            }
            if let Err(error) = stop_and_confirm_processes(executable) {
                emit(app, 28, "无法结束已安装程序，已取消更新");
                return Err(format!(
                    "无法确认已安装程序已经退出；更新未写入任何程序文件。{error}"
                ));
            }
        }
    }

    emit(app, 48, "写入程序文件…");

    write_payload_transactionally(&target_exe)?;

    // A legacy install is upgraded in place. Remove only the old, exact
    // product executable after the new payload has been verified.
    if legacy_exe.is_file() {
        std::fs::remove_file(&legacy_exe).map_err(|e| format!("清理旧版程序文件失败: {e}"))?;
    }

    std::fs::write(target_dir.join(INSTALL_MARKER_NAME), INSTALL_MARKER_CONTENT)
        .map_err(|e| format!("写入安装标记失败: {e}"))?;

    emit(app, 72, "创建快捷方式…");
    let report = create_shortcuts(&target_exe, &target_dir);

    emit(app, 90, "写入注册表…");
    let uninstaller = write_registry(&target_dir, &target_exe, &report.created);

    // Always drop a diagnostic log next to the exe so any "shortcut didn't
    // appear" report is debuggable without guesswork.
    write_install_log(&target_dir, updating, &report, &uninstaller);

    let done_msg = if !uninstaller.ready {
        "完成（新版卸载程序未能写入，请重试安装）"
    } else if report.created.is_empty() {
        "完成（未能创建快捷方式，详见 install.log）"
    } else if !report.errors.is_empty() {
        "完成（部分快捷方式未创建）"
    } else {
        "完成"
    };
    emit(app, 100, done_msg);
    Ok(InstallReport {
        shortcuts: report.created.len(),
        shortcut_errors: report.errors.clone(),
        uninstaller: uninstaller.ready,
        uninstaller_error: uninstaller.error,
    })
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// Outcome of the shortcut step: which .lnk files were created, plus any errors.
struct ShortcutReport {
    created: Vec<PathBuf>,
    errors: Vec<String>,
}

/// The per-user Start-Menu "Programs" folder.
#[cfg(windows)]
fn programs_dir() -> Option<PathBuf> {
    win_shortcut::known_folder_programs().or_else(env_programs)
}
#[cfg(not(windows))]
fn programs_dir() -> Option<PathBuf> {
    env_programs()
}

/// The user's Desktop folder (honors OneDrive redirection on Windows).
#[cfg(windows)]
fn desktop_dir() -> Option<PathBuf> {
    win_shortcut::known_folder_desktop().or_else(env_desktop)
}
#[cfg(not(windows))]
fn desktop_dir() -> Option<PathBuf> {
    env_desktop()
}

fn env_programs() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs"))
}

fn env_desktop() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|u| PathBuf::from(u).join("Desktop"))
}

/// Create the Start-Menu and Desktop shortcuts, reporting per-target results.
fn create_shortcuts(exe: &Path, workdir: &Path) -> ShortcutReport {
    let mut report = ShortcutReport {
        created: Vec::new(),
        errors: Vec::new(),
    };
    let targets: [(&str, Option<PathBuf>); 2] =
        [("开始菜单", programs_dir()), ("桌面", desktop_dir())];

    for (label, dir) in targets {
        let Some(dir) = dir else {
            report.errors.push(format!("{label}: 无法定位目标目录"));
            continue;
        };
        let _ = std::fs::create_dir_all(&dir);
        let lnk = dir.join(SHORTCUT_NAME);
        match make_shortcut(&lnk, exe, workdir) {
            Ok(()) => report.created.push(lnk),
            Err(e) => report.errors.push(format!("{label}: {e}")),
        }
    }
    report
}

/// Create one shortcut: COM first (reliable, Unicode-safe), PowerShell fallback.
fn make_shortcut(lnk: &Path, exe: &Path, workdir: &Path) -> Result<(), String> {
    let lnk = win32_compatible_path(lnk);
    let exe = win32_compatible_path(exe);
    let workdir = win32_compatible_path(workdir);

    #[cfg(windows)]
    let primary_err = match win_shortcut::create(&lnk, &exe, &workdir, &exe, DISPLAY_NAME) {
        Ok(()) if lnk.exists() => return Ok(()),
        Ok(()) => "COM 报告成功但未生成文件".to_string(),
        Err(e) => format!("COM: {e}"),
    };
    #[cfg(not(windows))]
    let primary_err = "非 Windows 平台".to_string();

    match powershell_shortcut(&lnk, &exe, &workdir) {
        Ok(()) => Ok(()),
        Err(ps_err) => Err(format!("{primary_err}；PowerShell: {ps_err}")),
    }
}

fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// Write `content` as UTF-8 **with a BOM** so PowerShell (and any Unicode-aware
/// reader) decodes it correctly regardless of the active console code page.
fn write_utf8_bom(path: &Path, content: &str) -> std::io::Result<()> {
    std::fs::write(path, utf8_bom(content))
}

fn write_utf8_bom_transactionally(path: &Path, content: &str) -> Result<(), String> {
    let bytes = utf8_bom(content);
    write_file_transactionally_with(path, &bytes, commit_staged_file)
}

fn utf8_bom(content: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(content.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(content.as_bytes());
    bytes
}

/// Last-ditch fallback: build the shortcut via WScript.Shell from a UTF-8 (BOM)
/// script file run with ExecutionPolicy Bypass, capturing the exit code/output
/// so a failure is no longer silent. Returns Ok only if the .lnk truly appears.
fn powershell_shortcut(lnk: &Path, exe: &Path, workdir: &Path) -> Result<(), String> {
    let script = format!(
        "$ErrorActionPreference='Stop';\
         $w=New-Object -ComObject WScript.Shell;\
         $s=$w.CreateShortcut('{lnk}');\
         $s.TargetPath='{exe}';\
         $s.WorkingDirectory='{wd}';\
         $s.IconLocation='{exe}';\
         $s.Save();",
        lnk = ps_quote(&lnk.to_string_lossy()),
        exe = ps_quote(&exe.to_string_lossy()),
        wd = ps_quote(&workdir.to_string_lossy()),
    );

    let mut tmp = std::env::temp_dir();
    tmp.push(format!("ngt_lnk_{}.ps1", std::process::id()));
    // UTF-8 BOM so PowerShell decodes CJK paths regardless of console code page.
    write_utf8_bom(&tmp, &script).map_err(|e| format!("写脚本失败: {e}"))?;

    let tmp_s = tmp.to_string_lossy().into_owned();
    let out = output_hidden(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &tmp_s,
        ],
    )
    .map_err(|e| format!("powershell 未启动: {e}"));
    let _ = std::fs::remove_file(&tmp);

    let out = out?;
    if out.status.success() && lnk.exists() {
        Ok(())
    } else {
        Err(format!(
            "退出码 {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

// ---------------------------------------------------------------------------
// In-process Shell COM shortcut creation (Windows)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win_shortcut {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    use windows::core::{Interface, Result, GUID, PCWSTR, PWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink,
        KF_FLAG_CREATE, KF_FLAG_DONT_VERIFY,
    };

    /// A NUL-terminated UTF-16 buffer for passing to wide Win32 APIs.
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// RAII guard for the COM apartment on the current thread.
    struct ComGuard {
        owned: bool,
    }
    impl ComGuard {
        fn new() -> Self {
            // SAFETY: balanced by CoUninitialize in Drop when we actually
            // performed a successful initialization on this thread.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            // S_OK / S_FALSE => we initialized and must uninitialize.
            // RPC_E_CHANGED_MODE => COM already up in another mode; usable, but
            // we must NOT uninitialize it.
            ComGuard { owned: hr.is_ok() }
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.owned {
                // SAFETY: matched with the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }

    fn known(id: *const GUID) -> Option<PathBuf> {
        // SAFETY: `id` points to a static FOLDERID GUID; the returned PWSTR is
        // freed with CoTaskMemFree exactly once.
        //
        // KF_FLAG_CREATE | KF_FLAG_DONT_VERIFY: return (and create) the *current*
        // redirected path even if the folder isn't materialized yet. With the
        // default (verify) flag, a fresh OneDrive-KFM profile whose redirected
        // Desktop/Programs folder doesn't exist on disk yet makes the API fail →
        // we'd silently fall back to the un-redirected %USERPROFILE%\Desktop and
        // drop the shortcut where the user can't see it. This keeps the
        // redirection-aware path authoritative.
        unsafe {
            let pw: PWSTR =
                SHGetKnownFolderPath(id, KF_FLAG_CREATE | KF_FLAG_DONT_VERIFY, None).ok()?;
            if pw.is_null() {
                return None;
            }
            let s = pw.to_string().ok();
            CoTaskMemFree(Some(pw.0 as *const c_void));
            s.map(PathBuf::from)
        }
    }

    /// The per-user Start-Menu "Programs" folder.
    pub fn known_folder_programs() -> Option<PathBuf> {
        known(&FOLDERID_Programs)
    }

    /// The user's Desktop folder (redirected target if OneDrive KFM is on).
    pub fn known_folder_desktop() -> Option<PathBuf> {
        known(&FOLDERID_Desktop)
    }

    /// Create a `.lnk` at `lnk` pointing at `target`, via IShellLinkW.
    pub fn create(
        lnk: &Path,
        target: &Path,
        workdir: &Path,
        icon: &Path,
        desc: &str,
    ) -> Result<()> {
        let _com = ComGuard::new();

        // Keep wide buffers alive for the whole unsafe block.
        let target_w = wide(&target.to_string_lossy());
        let wd_w = wide(&workdir.to_string_lossy());
        let icon_w = wide(&icon.to_string_lossy());
        let desc_w = wide(desc);
        let lnk_w = wide(&lnk.to_string_lossy());

        // SAFETY: COM is initialized for this thread; all PCWSTR pointers refer
        // to buffers that outlive each call; the IPersistFile cast is checked.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(PCWSTR(target_w.as_ptr()))?;
            link.SetWorkingDirectory(PCWSTR(wd_w.as_ptr()))?;
            link.SetIconLocation(PCWSTR(icon_w.as_ptr()), 0)?;
            // Description is cosmetic — don't fail the whole shortcut over it.
            let _ = link.SetDescription(PCWSTR(desc_w.as_ptr()));

            let persist: IPersistFile = link.cast()?;
            persist.Save(PCWSTR(lnk_w.as_ptr()), true)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Registry + uninstaller
// ---------------------------------------------------------------------------

struct UninstallerReport {
    ready: bool,
    error: Option<String>,
}

/// Refresh the GUI uninstaller and wire the best available entry into HKCU.
fn write_registry(dir: &Path, exe: &Path, created: &[PathBuf]) -> UninstallerReport {
    let set_sz = |value: &str, data: &str| {
        run_hidden(
            "reg",
            &[
                "add",
                UNINSTALL_KEY,
                "/v",
                value,
                "/t",
                "REG_SZ",
                "/d",
                data,
                "/f",
            ],
        )
    };
    // Remove any stale batch uninstaller from an older install.
    let _ = std::fs::remove_file(dir.join("uninstall.cmd"));

    // Atomically refresh the GUI uninstaller. A failed update preserves the
    // previous copy instead of truncating the only uninstall entry point.
    let uninstaller_exe = dir.join("uninstall.exe");
    let copy_result = write_current_executable_transactionally(&uninstaller_exe);
    let copied = copy_result.is_ok();
    let copy_error = copy_result.err();

    // Also refresh the PowerShell fallback transactionally.
    let uninstall_ps1 = dir.join("uninstall.ps1");
    let fallback_result =
        write_utf8_bom_transactionally(&uninstall_ps1, &uninstall_script(dir, created));
    let fallback_written = fallback_result.is_ok();
    let fallback_error = fallback_result.err();
    let registry_dir = win32_compatible_path(dir);
    let registry_exe = win32_compatible_path(exe);
    let registry_uninstaller = win32_compatible_path(&uninstaller_exe);
    let registry_uninstall_ps1 = win32_compatible_path(&uninstall_ps1);
    let cmdline = if copied {
        format!("\"{}\" --uninstall", registry_uninstaller.to_string_lossy())
    } else if fallback_written {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"",
            registry_uninstall_ps1.to_string_lossy()
        )
    } else {
        return UninstallerReport {
            ready: false,
            error: Some(format!(
                "GUI 卸载程序写入失败: {}；备用卸载脚本写入失败: {}",
                copy_error.as_deref().unwrap_or("未知错误"),
                fallback_error.as_deref().unwrap_or("未知错误"),
            )),
        };
    };

    let mut registry_ok = true;
    registry_ok &= set_sz("DisplayName", DISPLAY_NAME);
    registry_ok &= set_sz("DisplayVersion", VERSION);
    registry_ok &= set_sz("Publisher", PUBLISHER);
    registry_ok &= set_sz("InstallLocation", &registry_dir.to_string_lossy());
    registry_ok &= set_sz("DisplayIcon", &registry_exe.to_string_lossy());

    for (v, d) in [("NoModify", "1"), ("NoRepair", "1")] {
        registry_ok &= run_hidden(
            "reg",
            &[
                "add",
                UNINSTALL_KEY,
                "/v",
                v,
                "/t",
                "REG_DWORD",
                "/d",
                d,
                "/f",
            ],
        );
    }

    let registry_wired = registry_ok && set_sz("UninstallString", &cmdline);
    let ready = copied && registry_wired;
    let error = if ready {
        None
    } else if !copied {
        Some(format!(
            "新版 GUI 卸载程序写入失败: {}",
            copy_error.as_deref().unwrap_or("未知错误"),
        ))
    } else {
        Some("新版 GUI 卸载程序已写入，但卸载注册表项写入失败。".into())
    };
    if ready && registry_location_matches(LEGACY_UNINSTALL_KEY, dir) {
        // Do not leave the pre-rename product in Windows' uninstall list after
        // a successful migration. If the new entry was not ready, preserving
        // the legacy key keeps the old recovery path available. A separate
        // legacy installation at another path is intentionally untouched.
        let _ = run_hidden("reg", &["delete", LEGACY_UNINSTALL_KEY, "/f"]);
    }
    UninstallerReport { ready, error }
}

/// Build the PowerShell uninstaller. Deletes the exact `.lnk` paths we created
/// (which may be OneDrive-redirected) plus the legacy default locations and the
/// registry key, then removes the install directory from a detached child so the
/// still-running script (which lives inside that directory) does not block it.
/// The install dir is passed to the detached step via an environment variable to
/// avoid any command-line quoting/encoding pitfalls.
fn uninstall_script(dir: &Path, created: &[PathBuf]) -> String {
    let dir = win32_compatible_path(dir);
    let mut targets = String::new();
    for lnk in created {
        let lnk = win32_compatible_path(lnk);
        targets.push_str(&format!("  '{}',\r\n", ps_quote(&lnk.to_string_lossy())));
    }
    let detached_cleanup = format!(
        "while (Get-Process -Id $env:NGT_UNINSTALL_PID -ErrorAction SilentlyContinue) {{ Start-Sleep -Milliseconds 200 }}; $d=$env:NGT_RM; {} {}",
        owned_install_cleanup_commands("$d"),
        owned_cache_cleanup_commands(false),
    )
    .replace("\r\n", " ");

    format!(
        "$ErrorActionPreference = 'Stop'\r\n\
         {path_normalizer}\
         $targetExe = '{target_exe}'\r\n\
         $targetExe = Convert-NgtWin32Path $targetExe\r\n\
         try {{\r\n\
         \x20 $matching = @(Get-CimInstance Win32_Process -Filter \"Name='{exe}'\" | Where-Object {{ $_.ExecutablePath -and [StringComparer]::OrdinalIgnoreCase.Equals((Convert-NgtWin32Path $_.ExecutablePath),$targetExe) }})\r\n\
         \x20 $matching | ForEach-Object {{ $process = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; if ($process) {{ [void]$process.CloseMainWindow() }} }}\r\n\
         \x20 Start-Sleep -Milliseconds 400\r\n\
         \x20 $remaining = @(Get-CimInstance Win32_Process -Filter \"Name='{exe}'\" | Where-Object {{ $_.ExecutablePath -and [StringComparer]::OrdinalIgnoreCase.Equals((Convert-NgtWin32Path $_.ExecutablePath),$targetExe) }})\r\n\
         \x20 $remaining | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}\r\n\
         \x20 Start-Sleep -Milliseconds 400\r\n\
         \x20 $remaining = @(Get-CimInstance Win32_Process -Filter \"Name='{exe}'\" | Where-Object {{ $_.ExecutablePath -and [StringComparer]::OrdinalIgnoreCase.Equals((Convert-NgtWin32Path $_.ExecutablePath),$targetExe) }})\r\n\
         \x20 if ($remaining.Count -ne 0) {{ exit 1 }}\r\n\
         }} catch {{ exit 1 }}\r\n\
         $ErrorActionPreference = 'SilentlyContinue'\r\n\
         $env:NGT_RM = '{dir}'\r\n\
         $env:NGT_UNINSTALL_PID = [string]$PID\r\n\
         try {{\r\n\
         \x20 Start-Process -WindowStyle Hidden -FilePath 'powershell' -ArgumentList @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-WindowStyle','Hidden','-Command','{detached_cleanup}') -ErrorAction Stop | Out-Null\r\n\
         }} catch {{\r\n\
         \x20 exit 1\r\n\
         }}\r\n\
         $targets = @(\r\n\
         {targets}\
         \x20 (Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\{lnk}'),\r\n\
         \x20 (Join-Path $env:USERPROFILE 'Desktop\\{lnk}')\r\n\
         )\r\n\
         foreach ($t in $targets) {{ Remove-Item -LiteralPath $t -Force -ErrorAction SilentlyContinue }}\r\n\
         Remove-Item -LiteralPath '{registry}' -Recurse -Force -ErrorAction SilentlyContinue\r\n\
         $legacyRegistry = '{legacy_registry}'\r\n\
         $legacyInstall = (Get-ItemProperty -LiteralPath $legacyRegistry -Name InstallLocation -ErrorAction SilentlyContinue).InstallLocation\r\n\
         if ($legacyInstall -and [StringComparer]::OrdinalIgnoreCase.Equals((Convert-NgtWin32Path $legacyInstall),(Convert-NgtWin32Path $env:NGT_RM))) {{\r\n\
         \x20 Remove-Item -LiteralPath $legacyRegistry -Recurse -Force -ErrorAction SilentlyContinue\r\n\
         }}\r\n",
        target_exe = ps_quote(&dir.join(EXE_NAME).to_string_lossy()),
        path_normalizer = POWERSHELL_WIN32_PATH_FUNCTION,
        targets = targets,
        lnk = SHORTCUT_NAME,
        dir = ps_quote(&dir.to_string_lossy()),
        exe = EXE_NAME,
        registry = powershell_uninstall_key(),
        legacy_registry = powershell_legacy_uninstall_key(),
        detached_cleanup = ps_quote(&detached_cleanup),
    )
}

fn powershell_uninstall_key() -> String {
    format!(r"HKCU:\{UNINSTALL_SUBKEY}")
}

fn powershell_legacy_uninstall_key() -> String {
    format!(r"HKCU:\{LEGACY_UNINSTALL_SUBKEY}")
}

fn owned_cache_identifiers(delete_user_data: bool) -> Vec<&'static str> {
    let mut identifiers = INSTALLER_CACHE_IDENTIFIERS.to_vec();
    if delete_user_data {
        identifiers.extend(DESKTOP_CACHE_IDENTIFIERS);
    }
    identifiers
}

fn owned_install_cleanup_commands(dir_variable: &str) -> String {
    format!(
        "@('{exe}','{legacy_exe}','uninstall.exe','uninstall.ps1','uninstall.cmd','install.log','{marker}') | ForEach-Object {{\r\n\
         \x20 Remove-Item -LiteralPath (Join-Path {dir_variable} $_) -Force -ErrorAction SilentlyContinue\r\n\
         }}\r\n\
         Get-ChildItem -LiteralPath {dir_variable} -Force -File -ErrorAction SilentlyContinue | Where-Object {{\r\n\
         \x20 $_.Name -eq '.{exe}.previous' -or\r\n\
         \x20 $_.Name -eq '.uninstall.exe.previous' -or\r\n\
         \x20 $_.Name -eq '.uninstall.ps1.previous' -or\r\n\
         \x20 $_.Name -like '.{exe}.*.tmp' -or\r\n\
         \x20 $_.Name -like '.uninstall.exe.*.tmp' -or\r\n\
         \x20 $_.Name -like '.uninstall.ps1.*.tmp'\r\n\
         }} | ForEach-Object {{ Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }}\r\n\
         Remove-Item -LiteralPath {dir_variable} -Force -ErrorAction SilentlyContinue\r\n",
        exe = EXE_NAME,
        legacy_exe = LEGACY_EXE_NAME,
        marker = INSTALL_MARKER_NAME,
    )
}

fn owned_cache_cleanup_commands(delete_user_data: bool) -> String {
    let cache_roots = owned_cache_identifiers(delete_user_data)
        .into_iter()
        .map(|identifier| format!("  (Join-Path $env:LOCALAPPDATA '{}')", ps_quote(identifier)))
        .collect::<Vec<_>>()
        .join(",\r\n");
    format!(
        "if ($env:LOCALAPPDATA) {{\r\n\
         \x20 $cacheRoots = @(\r\n\
         {cache_roots}\r\n\
         \x20 )\r\n\
         \x20 foreach ($cache in $cacheRoots) {{\r\n\
         \x20\x20 for ($attempt = 0; $attempt -lt 20 -and (Test-Path -LiteralPath $cache); $attempt++) {{\r\n\
         \x20\x20\x20 Remove-Item -LiteralPath $cache -Recurse -Force -ErrorAction SilentlyContinue\r\n\
         \x20\x20\x20 if (Test-Path -LiteralPath $cache) {{ Start-Sleep -Milliseconds 250 }}\r\n\
         \x20\x20 }}\r\n\
         \x20 }}\r\n\
         }}\r\n"
    )
}

fn cleanup_script(dir: &Path, process_id: u32, delete_user_data: bool) -> String {
    let dir = win32_compatible_path(dir);
    let dir = ps_quote(&dir.to_string_lossy());
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\r\n\
         while (Get-Process -Id {process_id} -ErrorAction SilentlyContinue) {{ Start-Sleep -Milliseconds 200 }}\r\n\
         $dir = '{dir}'\r\n\
         {install_cleanup}\
         {cache_cleanup}\
         Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue\r\n",
        install_cleanup = owned_install_cleanup_commands("$dir"),
        cache_cleanup = owned_cache_cleanup_commands(delete_user_data),
    )
}

fn schedule_install_cleanup(dir: &Path, delete_user_data: bool) -> Result<(), String> {
    let process_id = std::process::id();
    let script_path = std::env::temp_dir().join(format!("ngt_uninstall_{process_id}.ps1"));
    write_utf8_bom(
        &script_path,
        &cleanup_script(dir, process_id, delete_user_data),
    )
    .map_err(|e| format!("无法创建卸载清理任务: {e}"))?;
    let script = script_path.to_string_lossy().into_owned();
    hidden("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
            &script,
        ])
        .spawn()
        .map_err(|e| format!("无法启动卸载清理任务: {e}"))?;
    Ok(())
}

/// Write a small diagnostic log next to the installed exe.
fn write_install_log(
    dir: &Path,
    updating: bool,
    report: &ShortcutReport,
    uninstaller: &UninstallerReport,
) {
    let mut log = String::new();
    log.push_str("Novel Generate Agent — install log\r\n");
    log.push_str(&format!("install dir : {}\r\n", dir.to_string_lossy()));
    log.push_str(&format!(
        "mode        : {}\r\n",
        if updating { "update" } else { "fresh" }
    ));
    log.push_str(&format!("start-menu  : {:?}\r\n", programs_dir()));
    log.push_str(&format!("desktop     : {:?}\r\n", desktop_dir()));
    log.push_str(&format!(
        "uninstaller : {}\r\n",
        if uninstaller.ready {
            "registered"
        } else {
            "FAILED to write"
        }
    ));
    if let Some(error) = &uninstaller.error {
        log.push_str(&format!("uninstall err: {error}\r\n"));
    }
    log.push_str("shortcuts created:\r\n");
    if report.created.is_empty() {
        log.push_str("  (none)\r\n");
    } else {
        for l in &report.created {
            log.push_str(&format!("  + {}\r\n", l.to_string_lossy()));
        }
    }
    if !report.errors.is_empty() {
        log.push_str("shortcut errors:\r\n");
        for e in &report.errors {
            log.push_str(&format!("  ! {e}\r\n"));
        }
    }
    let _ = std::fs::write(dir.join("install.log"), log);
}

/// Launch the freshly-installed app.
#[tauri::command]
fn launch(dir: String) -> Result<(), String> {
    let dir_path = validate_uninstall_target(Path::new(&dir))?;
    let exe = dir_path.join(EXE_NAME);
    Command::new(&exe)
        .current_dir(&dir_path)
        .spawn()
        .map_err(|e| format!("启动失败: {e}"))?;
    Ok(())
}

/// Uninstall the application: stop process, remove files, shortcuts, and registry.
#[tauri::command]
fn uninstall(delete_user_data: bool) -> Result<String, String> {
    // 1. Try multiple methods to find installation directory
    let install_dir = find_install_dir()?;
    let dir = PathBuf::from(&install_dir);

    if !dir.exists() {
        return Err(format!("安装目录不存在: {}", install_dir));
    }
    let dir = validate_uninstall_target(&dir)?;

    // 2. Stop only installed executable paths, preserving same-named portable
    // copies. A partially migrated legacy directory can contain both names.
    for executable in [dir.join(EXE_NAME), dir.join(LEGACY_EXE_NAME)] {
        if !executable.is_file() {
            continue;
        }
        if let Err(error) = stop_and_confirm_processes(&executable) {
            return Err(format!("无法确认已安装程序已经退出；卸载尚未开始。{error}"));
        }
    }

    // Preflight the detached cleanup before removing registry metadata or user
    // data. If PowerShell cannot be started, the installation remains recoverable.
    schedule_install_cleanup(&dir, delete_user_data)?;

    // 3. Delete shortcuts
    let mut deleted_shortcuts = Vec::new();
    if let Some(sm) = programs_dir() {
        let lnk = sm.join(SHORTCUT_NAME);
        if lnk.exists() && std::fs::remove_file(&lnk).is_ok() {
            deleted_shortcuts.push("开始菜单".to_string());
        }
    }
    if let Some(dt) = desktop_dir() {
        let lnk = dt.join(SHORTCUT_NAME);
        if lnk.exists() && std::fs::remove_file(&lnk).is_ok() {
            deleted_shortcuts.push("桌面".to_string());
        }
    }

    // 4. Delete the current key and, when it points at this directory, the
    // pre-rename legacy key as well.
    let _ = hidden("reg").args(["delete", UNINSTALL_KEY, "/f"]).output();
    if registry_location_matches(LEGACY_UNINSTALL_KEY, &dir) {
        let _ = hidden("reg")
            .args(["delete", LEGACY_UNINSTALL_KEY, "/f"])
            .output();
    }

    // 5. Delete user data if requested
    let user_data_msg = if delete_user_data {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let mut removed = 0usize;
            let mut failures = Vec::new();
            for identifier in [
                "com.novelgenerateagent.desktop",
                "com.novelgenerateteam.desktop",
            ] {
                let user_dir = PathBuf::from(&appdata).join(identifier);
                if user_dir.exists() {
                    match std::fs::remove_dir_all(&user_dir) {
                        Ok(()) => removed += 1,
                        Err(error) => failures.push(format!("{}: {error}", user_dir.display())),
                    }
                }
            }
            if failures.is_empty() {
                if removed > 0 {
                    "\n已删除用户数据（作品、会话、记忆）".to_string()
                } else {
                    "\n未发现需要删除的用户数据".to_string()
                }
            } else {
                format!("\n部分用户数据删除失败: {}", failures.join("；"))
            }
        } else {
            "\n无法定位用户数据目录，用户数据未删除".to_string()
        }
    } else {
        String::new()
    };

    let shortcut_msg = if deleted_shortcuts.is_empty() {
        String::new()
    } else {
        format!("\n已删除快捷方式: {}", deleted_shortcuts.join("、"))
    };

    Ok(format!(
        "卸载完成{}{}程序文件将在 2 秒后删除。",
        shortcut_msg, user_data_msg
    ))
}

/// Try multiple methods to find the installation directory.
fn find_install_dir() -> Result<String, String> {
    // Method 1: Read current and pre-rename HKCU uninstall keys.
    if let Some(path) = reg_read("InstallLocation") {
        if PathBuf::from(&path).join(EXE_NAME).is_file() {
            return Ok(path);
        }
    }
    if let Some(path) = reg_read_from(LEGACY_UNINSTALL_KEY, "InstallLocation") {
        let dir = PathBuf::from(&path);
        if dir.join(EXE_NAME).is_file() || dir.join(LEGACY_EXE_NAME).is_file() {
            return Ok(path);
        }
    }

    // Method 2: Check if running from install directory (uninstall.exe location)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let parent_path = parent.to_string_lossy().to_string();
            if parent.join(EXE_NAME).is_file() || parent.join(LEGACY_EXE_NAME).is_file() {
                return Ok(parent_path);
            }
        }
    }

    // Method 3: Check current and legacy default installation paths.
    let default_path = default_install_dir();
    if default_path.join(EXE_NAME).is_file() {
        return Ok(default_path.to_string_lossy().to_string());
    }
    let legacy_path = legacy_default_install_dir();
    if legacy_path.join(LEGACY_EXE_NAME).is_file() {
        return Ok(legacy_path.to_string_lossy().to_string());
    }

    Err("未找到安装目录。请手动删除安装文件。".to_string())
}

/// Detect if running as uninstall.exe (returns true) or installer.exe (false).
#[tauri::command]
fn is_uninstall_mode() -> bool {
    let executable_mode = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .map(|name| name.contains("uninstall"))
        .unwrap_or(false);
    executable_mode || std::env::args().any(|arg| arg == "--uninstall")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            detect_existing,
            default_dir,
            installer_version,
            install,
            launch,
            uninstall,
            is_uninstall_mode
        ])
        .run(tauri::generate_context!())
        .expect("error while running installer");
}

#[cfg(test)]
mod uninstall_tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ngt_installer_safety_{tag}_{}_{nonce}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn uninstall_ps1_is_unicode_safe_and_detached() {
        let dir = PathBuf::from(r"C:\Users\x\AppData\Local\Programs\NovelGenerateAgent");
        let created = vec![dir.join(SHORTCUT_NAME), dir.join("sub").join(SHORTCUT_NAME)];
        let s = uninstall_script(&dir, &created);

        // Deletes via PowerShell (Unicode-safe), references the CJK .lnk name,
        // and includes the exact created paths.
        assert!(s.contains("Remove-Item"), "must delete via Remove-Item");
        assert!(
            s.contains(SHORTCUT_NAME),
            "must reference CJK shortcut name"
        );
        for lnk in &created {
            assert!(
                s.contains(&lnk.to_string_lossy().into_owned()),
                "must list created path {}",
                lnk.display()
            );
        }
        // Install dir is removed from a DETACHED step via an env var (no quoting
        // pitfalls, and the running script can't block its own dir).
        assert!(s.contains("Start-Process"), "must remove dir detached");
        assert!(s.contains("$env:NGT_RM"), "dir passed via env var");
        assert!(s.contains("$env:NGT_UNINSTALL_PID = [string]$PID"));
        assert!(s.contains("Get-Process -Id $env:NGT_UNINSTALL_PID"));
        assert!(s.contains("-ErrorAction Stop"));
        let schedule = s.find("Start-Process").unwrap();
        let shortcut_delete = s.find("foreach ($t in $targets)").unwrap();
        let registry_delete = s.find("Remove-Item -LiteralPath 'HKCU:").unwrap();
        assert!(schedule < shortcut_delete);
        assert!(schedule < registry_delete);
        // The old, broken batch approach must be gone.
        assert!(!s.contains("rmdir"), "no in-place rmdir self-delete");
        assert!(!s.contains("chcp"), "no code-page batch hack");
        assert!(!s.contains("$env:NGT_RM -Recurse"));
        assert!(!s.contains("$d -Recurse"));
        assert!(s.contains(INSTALLER_CACHE_IDENTIFIERS[0]));
        assert!(s.contains(INSTALLER_CACHE_IDENTIFIERS[1]));
        assert!(!s.contains(DESKTOP_CACHE_IDENTIFIERS[0]));
        assert!(!s.contains(DESKTOP_CACHE_IDENTIFIERS[1]));
        assert!(s.contains(&format!(".{EXE_NAME}.previous")));
        assert!(s.contains(&format!(".{EXE_NAME}.*.tmp")));
    }

    #[test]
    fn uninstall_requires_owned_layout_and_main_executable() {
        let dir = tmp("marker");
        std::fs::write(dir.join("unrelated.txt"), "keep").unwrap();
        let error = validate_uninstall_target(&dir).unwrap_err();
        assert!(error.contains("安装标记"));

        std::fs::write(dir.join(INSTALL_MARKER_NAME), INSTALL_MARKER_CONTENT).unwrap();
        let error = validate_uninstall_target(&dir).unwrap_err();
        assert!(error.contains("主程序"));

        std::fs::write(dir.join(EXE_NAME), b"MZ").unwrap();
        validate_uninstall_target(&dir).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn install_rejects_nonempty_unowned_directory() {
        let dir = tmp("unowned");
        std::fs::write(dir.join("user-document.txt"), "keep").unwrap();
        let error = validate_install_target(&dir).unwrap_err();
        assert!(error.contains("不是空目录"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_executable_alone_does_not_claim_a_directory() {
        let dir = tmp("legacy_spoof");
        std::fs::write(dir.join(LEGACY_EXE_NAME), b"MZ").unwrap();

        assert!(!has_legacy_install_layout(&dir));
        let error = validate_install_target(&dir).unwrap_err();
        assert!(error.contains("不是空目录"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn install_accepts_verified_pre_rename_layout() {
        let dir = tmp("legacy_layout");
        std::fs::write(dir.join(LEGACY_EXE_NAME), b"MZ").unwrap();
        std::fs::write(dir.join("uninstall.ps1"), b"legacy uninstaller").unwrap();

        assert!(has_legacy_install_layout(&dir));
        let normalized = validate_install_target(&dir).unwrap();
        assert_eq!(
            normalized_path_text(&normalized),
            normalized_path_text(&std::fs::canonicalize(&dir).unwrap())
        );
        validate_uninstall_target(&dir).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn install_accepts_unmarked_current_layout_from_older_installer() {
        let dir = tmp("unmarked_current");
        std::fs::write(dir.join(EXE_NAME), b"MZ").unwrap();
        std::fs::write(dir.join("uninstall.exe"), b"MZ").unwrap();

        assert!(has_unmarked_current_install_layout(&dir));
        validate_install_target(&dir).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn install_rejects_parent_directory_traversal() {
        let dir = tmp("traversal");
        let candidate = dir.join("new-dir").join("..");
        let error = validate_install_target(&candidate).unwrap_err();
        assert!(error.contains(".."));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nonexistent_target_uses_canonical_existing_parent() {
        let parent = tmp("canonical_parent");
        let candidate = parent
            .join("missing-one")
            .join("missing-two")
            .join("new-app");
        let normalized = validate_install_target(&candidate).unwrap();
        assert_eq!(
            normalized_path_text(&normalized),
            normalized_path_text(
                &std::fs::canonicalize(&parent)
                    .unwrap()
                    .join("missing-one")
                    .join("missing-two")
                    .join("new-app")
            )
        );
        let _ = std::fs::remove_dir_all(parent);
    }

    #[cfg(windows)]
    #[test]
    fn reparse_parent_is_resolved_before_install() {
        let root = tmp("reparse_parent");
        let real = root.join("real");
        let link = root.join("link");
        std::fs::create_dir_all(&real).unwrap();
        if std::os::windows::fs::symlink_dir(&real, &link).is_err() {
            let _ = std::fs::remove_dir_all(root);
            return;
        }

        let normalized = normalize_install_target(&link.join("app")).unwrap();
        assert_eq!(
            normalized_path_text(&normalized),
            normalized_path_text(&std::fs::canonicalize(&real).unwrap().join("app"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_staged_commit_preserves_previous_executable() {
        let dir = tmp("transaction_failure");
        let target = dir.join(EXE_NAME);
        std::fs::write(&target, b"old-version").unwrap();

        let error = write_file_transactionally_with(
            &target,
            b"new-version",
            |_temporary, _target, _backup| Err("injected commit failure".into()),
        )
        .unwrap_err();
        assert!(error.contains("injected"));
        assert_eq!(std::fs::read(&target).unwrap(), b"old-version");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_length_corruption_restores_previous_file() {
        let dir = tmp("transaction_verification");
        let target = dir.join("uninstall.exe");
        std::fs::write(&target, b"old-version").unwrap();

        let error = write_file_transactionally_with(
            &target,
            b"new-version",
            |temporary, target, backup| {
                std::fs::rename(target, backup).unwrap();
                std::fs::write(target, b"bad-version").unwrap();
                std::fs::remove_file(temporary).unwrap();
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.contains("内容校验"));
        assert_eq!(std::fs::read(&target).unwrap(), b"old-version");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_length_corruption_removes_failed_fresh_file() {
        let dir = tmp("fresh_transaction_verification");
        let target = dir.join("uninstall.exe");

        write_file_transactionally_with(&target, b"new-version", |temporary, target, _backup| {
            std::fs::write(target, b"bad-version").unwrap();
            std::fs::remove_file(temporary).unwrap();
            Ok(())
        })
        .unwrap_err();
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn protected_roots_are_never_valid_uninstall_targets() {
        let root = PathBuf::from(r"C:\");
        assert!(validate_uninstall_target(&root)
            .unwrap_err()
            .contains("受保护"));

        if let Some(home) = std::env::var_os("USERPROFILE") {
            assert!(validate_uninstall_target(Path::new(&home))
                .unwrap_err()
                .contains("受保护"));
        }
    }

    #[test]
    fn cleanup_is_pid_scoped_and_non_recursive() {
        let dir = PathBuf::from(r"C:\safe install");
        let script = cleanup_script(&dir, 4242, false);
        assert!(script.contains("Get-Process -Id 4242"));
        assert!(script.contains(EXE_NAME));
        assert!(script.contains(LEGACY_EXE_NAME));
        assert!(script.contains(INSTALL_MARKER_NAME));
        assert!(script.contains(&format!(".{EXE_NAME}.previous")));
        assert!(script.contains(&format!(".{EXE_NAME}.*.tmp")));
        assert!(script.contains(".uninstall.exe.previous"));
        assert!(script.contains(".uninstall.exe.*.tmp"));
        assert!(script.contains("Remove-Item -LiteralPath $dir -Force"));
        assert!(!script.contains("Remove-Item -LiteralPath $dir -Recurse"));
        assert!(script.contains("Remove-Item -LiteralPath $cache -Recurse"));
        assert!(script.contains(INSTALLER_CACHE_IDENTIFIERS[0]));
        assert!(script.contains(INSTALLER_CACHE_IDENTIFIERS[1]));
        assert!(!script.contains(DESKTOP_CACHE_IDENTIFIERS[0]));
        assert!(!script.contains(DESKTOP_CACHE_IDENTIFIERS[1]));
    }

    #[test]
    fn cleanup_includes_desktop_cache_only_when_user_data_is_selected() {
        let dir = PathBuf::from(r"C:\safe install");
        let preserve = cleanup_script(&dir, 4242, false);
        let delete = cleanup_script(&dir, 4242, true);

        for identifier in DESKTOP_CACHE_IDENTIFIERS {
            assert!(!preserve.contains(identifier));
            assert!(delete.contains(identifier));
        }
        for identifier in INSTALLER_CACHE_IDENTIFIERS {
            assert!(preserve.contains(identifier));
            assert!(delete.contains(identifier));
        }
    }

    #[test]
    fn process_selection_is_scoped_to_exact_executable_path() {
        let installed = PathBuf::from(
            r"C:\Users\x\AppData\Local\Programs\NovelGenerateAgent\NovelGenerateAgent.exe",
        );
        let script = process_query_script(&installed);
        assert!(script.contains("ExecutablePath"));
        assert!(script.contains("$ErrorActionPreference='Stop'"));
        assert!(script.contains(&installed.to_string_lossy().into_owned()));
        assert!(!script.contains("/IM"));

        let fallback = uninstall_script(installed.parent().unwrap(), &[]);
        assert!(fallback.contains(&installed.to_string_lossy().into_owned()));
        assert!(!fallback.contains("Stop-Process -Name"));
    }

    #[test]
    fn process_selection_uses_legacy_executable_name_during_migration() {
        let legacy = PathBuf::from(
            r"C:\Users\x\AppData\Local\Programs\NovelGenerateTeam\NovelGenerateTeam.exe",
        );
        let script = process_query_script(&legacy);
        assert!(script.contains("Name='NovelGenerateTeam.exe'"));
        assert!(script.contains(&legacy.to_string_lossy().into_owned()));
    }

    #[test]
    fn process_query_output_rejects_malformed_process_ids() {
        assert_eq!(parse_process_ids(b"4242\r\n7\r\n").unwrap(), vec![4242, 7]);
        assert!(parse_process_ids(b"4242\r\nnot-a-pid\r\n").is_err());
    }

    #[test]
    fn extended_drive_path_matches_ordinary_win32_path() {
        let prefixed = PathBuf::from(
            r"\\?\C:\Users\x\AppData\Local\Programs\NovelGenerateAgent\NovelGenerateAgent.exe",
        );
        let ordinary = PathBuf::from(
            r"C:\Users\x\AppData\Local\Programs\NovelGenerateAgent\NovelGenerateAgent.exe",
        );

        assert_eq!(win32_compatible_path(&prefixed), ordinary);
        assert_eq!(
            normalized_path_text(&prefixed),
            normalized_path_text(&ordinary)
        );

        let script = process_query_script(&prefixed);
        assert!(script.contains(&ordinary.to_string_lossy().into_owned()));
        assert!(script.contains("Convert-NgtWin32Path $_.ExecutablePath"));
        assert!(script.contains(&format!(
            "$target=Convert-NgtWin32Path '{}'",
            ordinary.to_string_lossy()
        )));
    }

    #[test]
    fn extended_unc_path_preserves_unc_semantics() {
        let prefixed = PathBuf::from(r"\\?\UNC\server\share\NovelGenerateAgent.exe");
        let lowercase_prefix = PathBuf::from(r"\\?\unc\server\share\NovelGenerateAgent.exe");
        let ordinary = PathBuf::from(r"\\server\share\NovelGenerateAgent.exe");
        let device = PathBuf::from(r"\\?\Volume{1234}\NovelGenerateAgent.exe");

        assert_eq!(win32_compatible_path(&prefixed), ordinary);
        assert_eq!(win32_compatible_path(&lowercase_prefix), ordinary);
        assert_eq!(win32_compatible_path(&ordinary), ordinary);
        assert_eq!(win32_compatible_path(&device), device);
    }

    #[test]
    fn generated_uninstall_boundaries_use_ordinary_paths() {
        let prefixed_dir = PathBuf::from(r"\\?\C:\Apps\NovelGenerateAgent");
        let ordinary_dir = PathBuf::from(r"C:\Apps\NovelGenerateAgent");
        let prefixed_shortcut = PathBuf::from(r"\\?\C:\Users\x\Desktop\墨·创作.lnk");
        let ordinary_shortcut = PathBuf::from(r"C:\Users\x\Desktop\墨·创作.lnk");

        let uninstall = uninstall_script(&prefixed_dir, &[prefixed_shortcut]);
        assert!(uninstall.contains(&format!(
            "$targetExe = '{}'",
            ordinary_dir.join(EXE_NAME).to_string_lossy()
        )));
        assert!(uninstall.contains(&format!(
            "$env:NGT_RM = '{}'",
            ordinary_dir.to_string_lossy()
        )));
        assert!(uninstall.contains(&ordinary_shortcut.to_string_lossy().into_owned()));
        assert!(uninstall.contains("CloseMainWindow()"));
        assert!(uninstall.contains("Stop-Process -Id $_.ProcessId -Force"));
        assert!(uninstall.contains("if ($remaining.Count -ne 0) { exit 1 }"));

        let cleanup = cleanup_script(&prefixed_dir, 4242, false);
        assert!(cleanup.contains(&format!("$dir = '{}'", ordinary_dir.to_string_lossy())));
    }

    #[test]
    fn manifest_and_tauri_versions_match() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["version"].as_str(), Some(VERSION));
    }

    #[test]
    fn powershell_registry_path_has_one_provider_prefix() {
        let path = powershell_uninstall_key();
        assert_eq!(
            path,
            r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateAgent"
        );
        assert!(!path.contains(r"HKCU:\HKCU\"));

        let legacy = powershell_legacy_uninstall_key();
        assert_eq!(
            legacy,
            r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateTeam"
        );
        assert!(!legacy.contains(r"HKCU:\HKCU\"));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("ngt_inst_test_{}_{}", tag, std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn inspect_shortcut(lnk: &Path) -> Vec<String> {
        let inspect = format!(
            "$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
             $s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); \
             Write-Output $s.TargetPath; Write-Output $s.WorkingDirectory",
            ps_quote(&lnk.to_string_lossy())
        );
        let output = output_hidden(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", &inspect],
        )
        .expect("PowerShell should inspect the shortcut");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn assert_shortcut_paths(values: &[String], target: &Path, workdir: &Path) {
        assert_eq!(
            values.len(),
            2,
            "shortcut inspection should return two paths"
        );
        assert!(!values[0].starts_with(r"\\?\"));
        assert!(!values[1].starts_with(r"\\?\"));
        assert!(values[0].eq_ignore_ascii_case(&target.to_string_lossy()));
        assert!(values[1].eq_ignore_ascii_case(&workdir.to_string_lossy()));
    }

    fn assert_powershell_parses(tag: &str, script: &str) {
        let dir = tmp(tag);
        let script_path = dir.join("generated.ps1");
        write_utf8_bom(&script_path, script).unwrap();
        let parse = format!(
            "$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
             $tokens=$null; $errors=$null; \
             [System.Management.Automation.Language.Parser]::ParseFile('{}',[ref]$tokens,[ref]$errors) | Out-Null; \
             if ($errors.Count -ne 0) {{ $errors | ForEach-Object {{ Write-Output ($_.Extent.StartLineNumber.ToString()+':'+$_.Extent.StartColumnNumber.ToString()+': '+$_.Message+' ['+$_.Extent.Text+']') }}; exit 1 }}",
            ps_quote(&script_path.to_string_lossy())
        );
        let output = output_hidden(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", &parse],
        )
        .expect("PowerShell parser should start");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            output.status.success(),
            "generated PowerShell '{tag}' should parse:\nstdout: {}\nstderr: {}\nscript:\n{script}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[test]
    fn com_creates_lnk_with_cjk_name() {
        let dir = tmp("com");
        let target = dir.join("app.exe");
        std::fs::write(&target, b"MZ").unwrap(); // dummy target
        let lnk = dir.join(SHORTCUT_NAME); // CJK filename, the real case
        win_shortcut::create(&lnk, &target, &dir, &target, "测试快捷方式")
            .expect("COM shortcut creation should succeed");
        assert!(lnk.exists(), ".lnk file must exist");
        assert!(
            std::fs::metadata(&lnk).unwrap().len() > 0,
            ".lnk must not be empty"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_folders_resolve() {
        assert!(
            programs_dir().is_some(),
            "Start-Menu Programs should resolve"
        );
        assert!(desktop_dir().is_some(), "Desktop should resolve");
    }

    #[test]
    fn make_shortcut_accepts_extended_length_target_paths() {
        let dir = tmp("mk");
        let prefixed_dir = std::fs::canonicalize(&dir).unwrap();
        let ordinary_dir = win32_compatible_path(&prefixed_dir);
        let target = ordinary_dir.join("app.exe");
        std::fs::write(&target, b"MZ").unwrap();
        let prefixed_target = prefixed_dir.join("app.exe");
        let lnk = dir.join("shortcut.lnk");
        make_shortcut(&lnk, &prefixed_target, &prefixed_dir)
            .expect("make_shortcut should accept canonical extended-length paths");
        assert!(lnk.exists());

        let values = inspect_shortcut(&lnk);
        assert_shortcut_paths(&values, &target, &ordinary_dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn make_shortcut_converts_extended_unc_target_paths() {
        let dir = tmp("unc_shortcut");
        let lnk = dir.join("unc-shortcut.lnk");
        let prefixed_workdir = PathBuf::from(r"\\?\UNC\server\share\NovelGenerateAgent");
        let prefixed_target = prefixed_workdir.join(EXE_NAME);
        let ordinary_workdir = PathBuf::from(r"\\server\share\NovelGenerateAgent");
        let ordinary_target = ordinary_workdir.join(EXE_NAME);

        make_shortcut(&lnk, &prefixed_target, &prefixed_workdir)
            .expect("make_shortcut should convert extended UNC paths");
        let values = inspect_shortcut(&lnk);
        assert_shortcut_paths(&values, &ordinary_target, &ordinary_workdir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_uninstall_and_cleanup_scripts_parse() {
        let dir = PathBuf::from(r"\\?\C:\Apps\NovelGenerateAgent");
        let shortcut = PathBuf::from(r"\\?\C:\Users\x\Desktop\墨·创作.lnk");
        assert_powershell_parses("parse_uninstall", &uninstall_script(&dir, &[shortcut]));
        assert_powershell_parses("parse_cleanup", &cleanup_script(&dir, 4242, false));
    }

    #[test]
    fn atomic_commit_retries_a_transient_file_lock() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tmp("replace_retry");
        let target = dir.join("uninstall.exe");
        let temporary = dir.join(".uninstall.exe.next.tmp");
        let backup = dir.join(".uninstall.exe.previous");
        std::fs::write(&target, b"old-uninstaller").unwrap();
        std::fs::write(&temporary, b"new-uninstaller").unwrap();

        let locked = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&target)
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(locked);
        });

        commit_staged_file(&temporary, &target, &backup)
            .expect("a short sharing violation should be retried");
        release.join().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new-uninstaller");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old-uninstaller");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn process_query_matches_canonicalized_running_executable() {
        let dir = tmp("process_query");
        let other_dir = tmp("process_query_other");
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot should be set");
        let source = PathBuf::from(system_root).join(r"System32\ping.exe");
        let executable = dir.join(EXE_NAME);
        let other_executable = other_dir.join(EXE_NAME);
        std::fs::copy(&source, &executable).expect("test executable should be copied");
        std::fs::copy(source, &other_executable).expect("other test executable should be copied");

        let mut child = Command::new(&executable)
            .args(["127.0.0.1", "-n", "30", "-w", "1000"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("test executable should start");
        let mut other_child = Command::new(&other_executable)
            .args(["127.0.0.1", "-n", "30", "-w", "1000"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("other test executable should start");
        let canonical = std::fs::canonicalize(&executable).unwrap();
        let other_canonical = std::fs::canonicalize(&other_executable).unwrap();
        assert!(canonical.to_string_lossy().starts_with(r"\\?\"));

        let mut matched = false;
        let mut isolated = false;
        for _ in 0..10 {
            let target_ids = matching_process_ids(&canonical);
            let other_ids = matching_process_ids(&other_canonical);
            if let (Ok(target_ids), Ok(other_ids)) = (target_ids, other_ids) {
                matched = target_ids.contains(&child.id()) && other_ids.contains(&other_child.id());
                isolated =
                    !target_ids.contains(&other_child.id()) && !other_ids.contains(&child.id());
            }
            if matched && isolated {
                matched = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        let stop_result = stop_and_confirm_processes(&canonical);
        let exited = child.try_wait().unwrap().is_some();
        let other_still_running = other_child.try_wait().unwrap().is_none();
        if !exited {
            let _ = child.kill();
        }
        let _ = other_child.kill();
        let _ = child.wait();
        let _ = other_child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other_dir);
        assert!(
            matched,
            "CIM should match the canonical extended target path"
        );
        assert!(
            isolated,
            "same-named processes in other paths must be excluded"
        );
        assert!(stop_result.is_ok(), "the exact process should be stopped");
        assert!(exited, "the matching process should have exited");
        assert!(
            other_still_running,
            "the same-named process in another directory must remain running"
        );
    }
}
