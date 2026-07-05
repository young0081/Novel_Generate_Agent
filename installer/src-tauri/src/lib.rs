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

use std::path::{Path, PathBuf};
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
fn run_hidden(program: &str, args: &[&str]) {
    let _ = hidden(program).args(args).output();
}

/// Run a helper command silently and capture its output.
fn output_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    hidden(program).args(args).output()
}

/// The entire main app (a single self-contained Tauri exe), embedded.
const PAYLOAD: &[u8] = include_bytes!("../payload/NovelGenerateAgent.exe");

const EXE_NAME: &str = "NovelGenerateAgent.exe";
const VERSION: &str = "0.1.0";
const PUBLISHER: &str = "Novel Generate Agent";
const DISPLAY_NAME: &str = "Novel Generate Agent (墨·创作)";
const SHORTCUT_NAME: &str = "墨·创作.lnk";
const UNINSTALL_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NovelGenerateAgent";

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
}

fn local_appdata() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

fn default_install_dir() -> PathBuf {
    local_appdata().join("Programs").join("NovelGenerateAgent")
}

/// Read a REG_SZ value from the uninstall key, if present.
fn reg_read(value: &str) -> Option<String> {
    let out = output_hidden("reg", &["query", UNINSTALL_KEY, "/v", value]).ok()?;
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

/// Detect whether the app is already installed, and where.
#[tauri::command]
fn detect_existing() -> DetectResult {
    // 1) Registry InstallLocation from a previous install of this installer.
    if let Some(loc) = reg_read("InstallLocation") {
        if Path::new(&loc).join(EXE_NAME).exists() {
            return DetectResult {
                installed: true,
                path: loc,
                version: reg_read("DisplayVersion"),
            };
        }
    }
    // 2) The default per-user location on disk.
    let def = default_install_dir();
    if def.join(EXE_NAME).exists() {
        return DetectResult {
            installed: true,
            path: def.to_string_lossy().into_owned(),
            version: reg_read("DisplayVersion"),
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
    let target_dir = PathBuf::from(dir);
    let target_exe = target_dir.join(EXE_NAME);
    let updating = target_exe.exists();

    emit(app, 6, if updating { "准备更新…" } else { "准备安装…" });
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("创建目录失败: {e}"))?;

    // Stop a running instance so the exe can be replaced (update-in-place).
    // Enhanced: poll to ensure the process actually exits.
    emit(app, 22, "结束正在运行的旧版本…");
    run_hidden("taskkill", &["/F", "/IM", EXE_NAME]);

    // Poll for up to 5 seconds to ensure the process is gone.
    let proc_name = EXE_NAME.trim_end_matches(".exe");
    for attempt in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        // Check if the process still exists via tasklist
        if let Ok(out) = output_hidden("tasklist", &["/FI", &format!("IMAGENAME eq {}", EXE_NAME), "/NH"]) {
            let text = String::from_utf8_lossy(&out.stdout);
            if !text.contains(proc_name) {
                break; // process is gone
            }
        }
        if attempt == 9 {
            // Still running after 5s — warn but proceed (user might have force-closed)
            emit(app, 28, "进程仍在运行，尝试强制覆盖…");
        }
    }

    emit(app, 48, "写入程序文件…");

    // Attempt 1: direct write
    let write_result = std::fs::write(&target_exe, PAYLOAD);

    // If it failed due to file-in-use, try deleting first then retry
    if let Err(ref e) = write_result {
        if e.kind() == std::io::ErrorKind::PermissionDenied && updating {
            emit(app, 52, "文件占用中，删除旧版本后重试…");
            let _ = std::fs::remove_file(&target_exe);
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::fs::write(&target_exe, PAYLOAD).map_err(|e2| format!("覆盖程序失败: {e2}"))?;
        } else {
            return Err(format!("写入程序失败: {e}"));
        }
    }

    emit(app, 72, "创建快捷方式…");
    let report = create_shortcuts(&target_exe, &target_dir);

    emit(app, 90, "写入注册表…");
    let uninstaller_ok = write_registry(&target_dir, &target_exe, &report.created);

    // Always drop a diagnostic log next to the exe so any "shortcut didn't
    // appear" report is debuggable without guesswork.
    write_install_log(&target_dir, updating, &report, uninstaller_ok);

    let done_msg = if report.created.is_empty() {
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
        uninstaller: uninstaller_ok,
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
    #[cfg(windows)]
    let primary_err = match win_shortcut::create(lnk, exe, workdir, exe, DISPLAY_NAME) {
        Ok(()) if lnk.exists() => return Ok(()),
        Ok(()) => "COM 报告成功但未生成文件".to_string(),
        Err(e) => format!("COM: {e}"),
    };
    #[cfg(not(windows))]
    let primary_err = "非 Windows 平台".to_string();

    match powershell_shortcut(lnk, exe, workdir) {
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
    let mut bytes = Vec::with_capacity(content.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(content.as_bytes());
    std::fs::write(path, &bytes)
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
        IShellLinkW, SHGetKnownFolderPath, ShellLink, FOLDERID_Desktop, FOLDERID_Programs,
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

/// Write the uninstall registry entry. Returns whether the uninstaller script
/// was written and wired up (so the caller can record it in the log).
fn write_registry(dir: &Path, exe: &Path, created: &[PathBuf]) -> bool {
    let set_sz = |value: &str, data: &str| {
        run_hidden(
            "reg",
            &["add", UNINSTALL_KEY, "/v", value, "/t", "REG_SZ", "/d", data, "/f"],
        );
    };
    set_sz("DisplayName", DISPLAY_NAME);
    set_sz("DisplayVersion", VERSION);
    set_sz("Publisher", PUBLISHER);
    set_sz("InstallLocation", &dir.to_string_lossy());
    set_sz("DisplayIcon", &exe.to_string_lossy());

    for (v, d) in [("NoModify", "1"), ("NoRepair", "1")] {
        run_hidden(
            "reg",
            &["add", UNINSTALL_KEY, "/v", v, "/t", "REG_DWORD", "/d", d, "/f"],
        );
    }

    // Remove any stale batch uninstaller from an older install.
    let _ = std::fs::remove_file(dir.join("uninstall.cmd"));

    // Create a GUI uninstaller: copy installer.exe as uninstall.exe in the install dir.
    // When launched with --uninstall flag, the frontend shows uninstall UI.
    let installer_path = std::env::current_exe().ok();
    let uninstaller_exe = dir.join("uninstall.exe");
    let copied = if let Some(src) = installer_path {
        std::fs::copy(&src, &uninstaller_exe).is_ok()
    } else {
        false
    };

    // Also create a PowerShell fallback uninstaller
    let uninstall_ps1 = dir.join("uninstall.ps1");
    if write_utf8_bom(&uninstall_ps1, &uninstall_script(dir, created)).is_ok() {
        // Register the GUI uninstaller if available, otherwise the PowerShell one
        let cmdline = if copied {
            format!("\"{}\" --uninstall", uninstaller_exe.to_string_lossy())
        } else {
            format!(
                "powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"",
                uninstall_ps1.to_string_lossy()
            )
        };
        set_sz("UninstallString", &cmdline);
        true
    } else {
        false
    }
}

/// Build the PowerShell uninstaller. Deletes the exact `.lnk` paths we created
/// (which may be OneDrive-redirected) plus the legacy default locations and the
/// registry key, then removes the install directory from a detached child so the
/// still-running script (which lives inside that directory) does not block it.
/// The install dir is passed to the detached step via an environment variable to
/// avoid any command-line quoting/encoding pitfalls.
fn uninstall_script(dir: &Path, created: &[PathBuf]) -> String {
    let mut targets = String::new();
    for lnk in created {
        targets.push_str(&format!("  '{}',\r\n", ps_quote(&lnk.to_string_lossy())));
    }

    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\r\n\
         Stop-Process -Name '{proc}' -Force\r\n\
         Start-Sleep -Milliseconds 400\r\n\
         $targets = @(\r\n\
         {targets}\
         \x20 (Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\{lnk}'),\r\n\
         \x20 (Join-Path $env:USERPROFILE 'Desktop\\{lnk}')\r\n\
         )\r\n\
         foreach ($t in $targets) {{ Remove-Item -LiteralPath $t -Force -ErrorAction SilentlyContinue }}\r\n\
         Remove-Item -LiteralPath 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\NovelGenerateAgent' -Recurse -Force -ErrorAction SilentlyContinue\r\n\
         $env:NGT_RM = '{dir}'\r\n\
         Start-Process -WindowStyle Hidden -FilePath 'powershell' -ArgumentList @('-NoProfile','-WindowStyle','Hidden','-Command','Start-Sleep -Seconds 2; Remove-Item -LiteralPath $env:NGT_RM -Recurse -Force -ErrorAction SilentlyContinue')\r\n",
        proc = EXE_NAME.trim_end_matches(".exe"),
        targets = targets,
        lnk = SHORTCUT_NAME,
        dir = ps_quote(&dir.to_string_lossy()),
    )
}

/// Write a small diagnostic log next to the installed exe.
fn write_install_log(dir: &Path, updating: bool, report: &ShortcutReport, uninstaller_ok: bool) {
    let mut log = String::new();
    log.push_str("Novel Generate Agent — install log\r\n");
    log.push_str(&format!("install dir : {}\r\n", dir.to_string_lossy()));
    log.push_str(&format!("mode        : {}\r\n", if updating { "update" } else { "fresh" }));
    log.push_str(&format!("start-menu  : {:?}\r\n", programs_dir()));
    log.push_str(&format!("desktop     : {:?}\r\n", desktop_dir()));
    log.push_str(&format!(
        "uninstaller : {}\r\n",
        if uninstaller_ok { "uninstall.ps1 written" } else { "FAILED to write" }
    ));
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
    let exe = PathBuf::from(&dir).join(EXE_NAME);
    Command::new(&exe)
        .current_dir(&dir)
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

    // 2. Stop running process
    let _ = hidden("taskkill")
        .args(&["/F", "/IM", EXE_NAME])
        .output();

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

    // 4. Delete registry key
    let key_path = format!("HKCU\\{}", UNINSTALL_KEY);
    let _ = hidden("reg")
        .args(&["delete", &key_path, "/f"])
        .output();

    // 5. Delete user data if requested
    let user_data_msg = if delete_user_data {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        let user_dir = PathBuf::from(appdata).join("com.novelgenerateteam.desktop");
        if user_dir.exists() {
            let _ = std::fs::remove_dir_all(&user_dir);
            "\n已删除用户数据（作品、会话、记忆）".to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // 6. Create a self-deleting batch script
    // We can't delete the directory while running from it, so we create a detached
    // batch file that waits for this process to exit, then deletes everything.
    let batch_path = std::env::temp_dir().join("ngt_uninstall.cmd");
    let batch_content = format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist | find /i \"uninstall.exe\" >nul 2>&1\r\n\
         if not errorlevel 1 (\r\n\
         \x20\x20timeout /t 1 /nobreak >nul\r\n\
         \x20\x20goto wait\r\n\
         )\r\n\
         rd /s /q \"{}\"\r\n\
         del \"%~f0\"\r\n",
        dir.display()
    );

    if std::fs::write(&batch_path, batch_content).is_ok() {
        // Launch the batch file detached (no window)
        let _ = hidden("cmd")
            .args(&["/C", "start", "/B", &batch_path.to_string_lossy()])
            .spawn();
    }

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
    // Method 1: Check registry with PowerShell
    let ps_script = format!(
        r#"(Get-ItemProperty -Path 'HKCU:\{}' -Name InstallLocation -ErrorAction SilentlyContinue).InstallLocation"#,
        UNINSTALL_KEY
    );
    if let Ok(output) = hidden("powershell")
        .args(&["-NoProfile", "-Command", &ps_script])
        .output()
    {
        let path = String::from_utf8_lossy(&output.stdout)
            .trim()
            .replace("\r", "")
            .replace("\n", "");
        if !path.is_empty() && PathBuf::from(&path).exists() {
            return Ok(path);
        }
    }

    // Method 2: Check if running from install directory (uninstall.exe location)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let parent_path = parent.to_string_lossy().to_string();
            // Check if this looks like our install dir (has NovelGenerateAgent.exe)
            if parent.join(EXE_NAME).exists() {
                return Ok(parent_path);
            }
        }
    }

    // Method 3: Check default installation path
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        let default_path = PathBuf::from(localappdata).join("Programs").join("NovelGenerateAgent");
        if default_path.exists() && default_path.join(EXE_NAME).exists() {
            return Ok(default_path.to_string_lossy().to_string());
        }
    }

    Err("未找到安装目录。请手动删除安装文件。".to_string())
}

/// Detect if running as uninstall.exe (returns true) or installer.exe (false).
#[tauri::command]
fn is_uninstall_mode() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .map(|name| name.contains("uninstall"))
        .unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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

    #[test]
    fn uninstall_ps1_is_unicode_safe_and_detached() {
        let dir = PathBuf::from(r"C:\Users\x\AppData\Local\Programs\NovelGenerateTeam");
        let created = vec![dir.join(SHORTCUT_NAME), dir.join("sub").join(SHORTCUT_NAME)];
        let s = uninstall_script(&dir, &created);

        // Deletes via PowerShell (Unicode-safe), references the CJK .lnk name,
        // and includes the exact created paths.
        assert!(s.contains("Remove-Item"), "must delete via Remove-Item");
        assert!(s.contains(SHORTCUT_NAME), "must reference CJK shortcut name");
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
        // The old, broken batch approach must be gone.
        assert!(!s.contains("rmdir"), "no in-place rmdir self-delete");
        assert!(!s.contains("chcp"), "no code-page batch hack");
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
        assert!(programs_dir().is_some(), "Start-Menu Programs should resolve");
        assert!(desktop_dir().is_some(), "Desktop should resolve");
    }

    #[test]
    fn make_shortcut_succeeds_and_file_appears() {
        let dir = tmp("mk");
        let target = dir.join("app.exe");
        std::fs::write(&target, b"MZ").unwrap();
        let lnk = dir.join("shortcut.lnk");
        make_shortcut(&lnk, &target, &dir).expect("make_shortcut should succeed");
        assert!(lnk.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
