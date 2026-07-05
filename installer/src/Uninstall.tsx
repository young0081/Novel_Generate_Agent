import { useState } from "react";
import { uninstall } from "./lib/installer";
import { closeWindow, minimizeWindow } from "./lib/win";

/* =========================================================================
   墨 · 创作 — 卸载程序
   A simplified uninstall UI using the same scroll aesthetic.
   ========================================================================= */

type Step = "confirm" | "uninstalling" | "done";

function Chrome() {
  return (
    <div className="chrome">
      <button className="chrome-btn" title="最小化" aria-label="最小化"
        onClick={() => void minimizeWindow()}>
        <svg viewBox="0 0 12 12" width="11" height="11"><line x1="2.5" y1="6" x2="9.5" y2="6"
          stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" /></svg>
      </button>
      <button className="chrome-btn chrome-btn--x" title="关闭" aria-label="关闭"
        onClick={() => void closeWindow()}>
        <svg viewBox="0 0 12 12" width="11" height="11"><path d="M3 3l6 6M9 3l-6 6"
          stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" /></svg>
      </button>
    </div>
  );
}

function CarveSeal({ char = "拆", size = 100 }: { char?: string; size?: number }) {
  return (
    <div className="seal" style={{ width: size, height: size }} aria-hidden="true">
      <div className="seal-stone">
        <span className="seal-carved" style={{ clipPath: "inset(0)" }}>{char}</span>
        <span className="seal-frame" />
      </div>
    </div>
  );
}

export default function UninstallApp() {
  const [step, setStep] = useState<Step>("confirm");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [deleteUserData, setDeleteUserData] = useState(false);

  const handleUninstall = async () => {
    setStep("uninstalling");
    setError(null);
    try {
      const result = await uninstall(deleteUserData);
      setMessage(result);
      setStep("done");
      // Auto-close after 1.5 seconds to allow batch script to delete directory
      setTimeout(() => {
        void closeWindow();
      }, 1500);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
      setStep("confirm");
    }
  };

  return (
    <div className="scroll">
      <div className="paper-grain" aria-hidden="true" />
      <div className="stage-drag" data-tauri-drag-region aria-hidden="true" />
      <Chrome />

      <main className="stage">
        {step === "confirm" && (
          <section className="panel panel--welcome">
            <div style={{ textAlign: "center", marginBottom: "2rem" }}>
              <CarveSeal char="别" size={80} />
            </div>
            <h1 className="lede reveal r2">确认卸载？</h1>
            <p className="dek reveal r3">
              将删除「墨·创作」的所有程序文件和快捷方式。
            </p>

            <div className="reveal r3" style={{ marginTop: "1.5rem" }}>
              <button
                onClick={() => setDeleteUserData(!deleteUserData)}
                style={{
                  display: "flex",
                  alignItems: "flex-start",
                  gap: "0.75rem",
                  width: "100%",
                  padding: "1rem",
                  border: `1.5px solid ${deleteUserData ? 'var(--cn)' : 'var(--paper-edge)'}`,
                  borderRadius: "8px",
                  background: deleteUserData ? 'var(--on-cn)' : 'transparent',
                  cursor: "pointer",
                  transition: "all 0.2s var(--e-out)",
                  fontFamily: "var(--sans)",
                  fontSize: "0.95rem",
                  textAlign: "left",
                }}
              >
                <div style={{
                  width: "20px",
                  height: "20px",
                  border: `2px solid ${deleteUserData ? 'var(--cn)' : 'var(--ink-3)'}`,
                  borderRadius: "4px",
                  background: deleteUserData ? 'var(--cn)' : 'transparent',
                  display: "grid",
                  placeItems: "center",
                  flexShrink: 0,
                  transition: "all 0.2s var(--e-out)",
                }}>
                  {deleteUserData && (
                    <svg width="12" height="10" viewBox="0 0 12 10" fill="none">
                      <path d="M1 5L4.5 8.5L11 1.5" stroke="var(--on-cn)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
                    </svg>
                  )}
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontWeight: 500, color: "var(--ink)", marginBottom: "0.25rem" }}>
                    同时删除用户数据
                  </div>
                  <div style={{ fontSize: "0.85rem", color: "var(--ink-3)", lineHeight: 1.5 }}>
                    包括所有作品、会话记录、知识库内容和记忆
                  </div>
                </div>
              </button>
            </div>

            {error && <p className="errline">{error}</p>}
            <div className="row row--split reveal r4">
              <button className="ink-btn ink-btn--ghost" onClick={() => void closeWindow()}>
                取消
              </button>
              <button className="ink-btn ink-btn--solid" onClick={handleUninstall}>
                确认卸载
              </button>
            </div>
          </section>
        )}

        {step === "uninstalling" && (
          <section className="panel panel--install">
            <div style={{ textAlign: "center", marginBottom: "2rem" }}>
              <CarveSeal char="拆" size={100} />
            </div>
            <h2 className="title">正在卸载…</h2>
            <p className="install-msg">请稍候，正在移除程序文件</p>
          </section>
        )}

        {step === "done" && (
          <section className="panel panel--done">
            <div style={{ textAlign: "center", marginBottom: "2rem" }}>
              <CarveSeal char="別" size={100} />
            </div>
            <h2 className="title reveal r2">卸载完成</h2>
            <p className="dek reveal r3" style={{ whiteSpace: "pre-line" }}>{message}</p>
            <p className="dek reveal r3" style={{ marginTop: "1rem", fontSize: "0.9rem" }}>
              如需重新安装，请运行安装程序。
            </p>
            <div className="row reveal r4">
              <button className="ink-btn ink-btn--solid" onClick={() => void closeWindow()}>
                关闭
              </button>
            </div>
          </section>
        )}
      </main>

      <aside className="band" aria-label="卸载进程">
        <div className="band-wordmark">
          <span className="wm-seal">墨</span>
          <span className="wm-rest">創作</span>
        </div>
        <div className="band-foot">卸载</div>
      </aside>
    </div>
  );
}
