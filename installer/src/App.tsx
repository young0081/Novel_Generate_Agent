import { useCallback, useEffect, useRef, useState } from "react";
import {
  detectExisting,
  install as runInstall,
  installerVersion,
  launch as runLaunch,
  type DetectResult,
  type InstallReport,
} from "./lib/installer";
import { closeWindow, minimizeWindow } from "./lib/win";

/* =========================================================================
   墨 · 创作 — 安装《立轴 · 钤印》
   A right-to-left hanging-scroll installer. The right vermilion band carries
   the vertical wordmark + four-rite tracker; the left paper stage carries
   left-aligned editorial content. Install progress is rendered as a seal being
   carved bottom-up, then stamped at completion. Fixed 760×520 frameless window.
   ========================================================================= */

type Step = "welcome" | "path" | "installing" | "done";
const ORDER: Step[] = ["welcome", "path", "installing", "done"];

const RITES: { key: Step; label: string }[] = [
  { key: "welcome", label: "缘起" },
  { key: "path", label: "择址" },
  { key: "installing", label: "镌刻" },
  { key: "done", label: "钤印" },
];

function reduced(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/* ---- glyph icons (hairline, no rounded-icon-chrome) --------------------- */
function GlyphArrow({ back = false }: { back?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"
      style={back ? { transform: "scaleX(-1)" } : undefined}>
      <path d="M4 12h15M13 6l6 6-6 6" fill="none" stroke="currentColor"
        strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
function GlyphFolder() {
  return (
    <svg viewBox="0 0 24 24" width="17" height="17" aria-hidden="true">
      <path d="M3 6.5C3 5.7 3.6 5 4.5 5h4.1c.4 0 .8.16 1 .44L11 6.7h8.5c.8 0 1.5.7 1.5 1.5v9.3c0 .8-.7 1.5-1.5 1.5h-15C3.6 19 3 18.3 3 17.5z"
        fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
    </svg>
  );
}

/* ---- CarveSeal: the signature mechanic ----------------------------------
   A vermilion relief seal. `reveal` (0..1) wipes the carved white glyph in
   from the bottom — the chisel working upward. `stamped` plays the press.   */
function CarveSeal({
  reveal,
  char = "創",
  size = 132,
  stamped = false,
  dim = false,
}: {
  reveal: number;
  char?: string;
  size?: number;
  stamped?: boolean;
  dim?: boolean;
}) {
  const r = Math.max(0, Math.min(1, reveal));
  // carved glyph clip: a rising horizontal band (inset from the bottom)
  const cut = `inset(${(1 - r) * 100}% 0 0 0)`;
  return (
    <div
      className={`seal ${stamped ? "is-stamped" : ""} ${dim ? "is-dim" : ""}`}
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <div className="seal-stone">
        {/* faint full impression behind, so the stone never looks empty */}
        <span className="seal-ghost">{char}</span>
        {/* carved (knocked-out white) glyph, revealed bottom-up */}
        <span className="seal-carved" style={{ clipPath: cut, WebkitClipPath: cut }}>
          {char}
        </span>
        <span className="seal-frame" />
        {/* a thin chisel line riding the reveal edge while carving */}
        {r > 0.02 && r < 0.99 && (
          <span className="seal-chisel" style={{ bottom: `${r * 100}%` }} />
        )}
      </div>
      {/* ink-bleed halo, only meaningful once stamped */}
      <span className="seal-bleed" />
    </div>
  );
}

/* ---- the band: vertical wordmark + four-rite tracker -------------------- */
function ScrollBand({ step }: { step: Step }) {
  const cur = ORDER.indexOf(step);
  return (
    <aside className="band" aria-label="安装进程">
      <div className="band-wordmark">
        <span className="wm-seal">墨</span>
        <span className="wm-rest">創作</span>
      </div>
      <ol className="rites">
        {RITES.map((rt, i) => {
          const state =
            i === cur ? "active" : i < cur ? "done" : "todo";
          return (
            <li key={rt.key} className={`rite rite--${state}`}>
              <span className="rite-mark" aria-hidden="true">
                {i < cur ? "·" : i + 1}
              </span>
              <span className="rite-label">{rt.label}</span>
            </li>
          );
        })}
      </ol>
      <div className="band-foot">NGT</div>
    </aside>
  );
}

/* ---- window controls (float over the band's top) ----------------------- */
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

export default function App() {
  const [step, setStep] = useState<Step>("welcome");
  const [detect, setDetect] = useState<DetectResult | null>(null);
  const [version, setVersion] = useState("");
  const [dir, setDir] = useState("");
  const [percent, setPercent] = useState(0);
  const [message, setMessage] = useState("正在准备…");
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<InstallReport | null>(null);
  const busy = useRef(false);

  const isUpdate = detect?.installed === true;

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [v, d] = await Promise.all([installerVersion(), detectExisting()]);
        if (!alive) return;
        setVersion(v);
        setDetect(d);
        setDir(d.path);
      } catch {
        if (!alive) return;
        setVersion((p) => p || "0.1.0");
        setDetect({ installed: false, path: "", version: null });
      }
    })();
    return () => { alive = false; };
  }, []);

  const startInstall = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    setError(null);
    setPercent(4);
    setMessage("正在准备安装环境…");
    setStep("installing");
    try {
      const rep = await runInstall(dir, (p) => {
        setPercent(p.percent);
        if (p.message) setMessage(p.message);
      });
      setReport(rep);
      setPercent(100);
      setStep("done");
    } catch (err) {
      setError(
        typeof err === "string" ? err
          : err instanceof Error ? err.message
          : "安装过程中出现未知错误",
      );
    } finally {
      busy.current = false;
    }
  }, [dir]);

  const onLaunch = useCallback(async () => {
    try { await runLaunch(dir); } catch { /* ignore */ }
    finally { await closeWindow(); }
  }, [dir]);

  return (
    <div className="scroll">
      <div className="paper-grain" aria-hidden="true" />
      <div className="stage-drag" data-tauri-drag-region aria-hidden="true" />
      <Chrome />

      <main className="stage" key={step}>
        {step === "welcome" && (
          <Welcome
            version={version}
            isUpdate={isUpdate}
            oldVersion={detect?.version ?? null}
            onNext={() => setStep("path")}
          />
        )}
        {step === "path" && (
          <PathStep
            dir={dir}
            setDir={setDir}
            isUpdate={isUpdate}
            oldVersion={detect?.version ?? null}
            version={version}
            onBack={() => setStep("welcome")}
            onInstall={() => void startInstall()}
          />
        )}
        {step === "installing" && (
          <Installing
            percent={percent}
            message={message}
            error={error}
            onRetry={() => void startInstall()}
            onBack={() => { setError(null); setStep("path"); }}
          />
        )}
        {step === "done" && (
          <Done
            isUpdate={isUpdate}
            version={version}
            shortcutWarning={report && report.shortcuts === 0 ? dir : null}
            onLaunch={() => void onLaunch()}
            onClose={() => void closeWindow()}
          />
        )}
      </main>

      <ScrollBand step={step} />
    </div>
  );
}

/* ---- Step 缘起 / Welcome ------------------------------------------------- */
function Welcome({
  version, isUpdate, oldVersion, onNext,
}: {
  version: string; isUpdate: boolean; oldVersion: string | null; onNext: () => void;
}) {
  return (
    <section className="panel panel--welcome">
      <p className="kicker reveal r1">团队协作 · 同人小说 AI 创作工坊</p>
      <h1 className="lede reveal r2">
        执笔，<br />自此处<em>落墨</em>。
      </h1>
      <p className="dek reveal r3">
        一方砚、一支笔、一座可与你并肩成稿的 AI 创作内核。
        {isUpdate
          ? `　检测到已安装 ${oldVersion ? "v" + oldVersion : "旧版"}，将原地更新。`
          : "　全新安装，片刻即成。"}
      </p>
      <div className="row reveal r4">
        <button className="ink-btn ink-btn--solid" onClick={onNext}>
          {isUpdate ? `更新至 v${version}` : "开始安装"}
          <GlyphArrow />
        </button>
        {version && <span className="ver">v{version}</span>}
      </div>
    </section>
  );
}

/* ---- Step 择址 / Path ---------------------------------------------------- */
function PathStep({
  dir, setDir, isUpdate, oldVersion, version, onBack, onInstall,
}: {
  dir: string; setDir: (v: string) => void; isUpdate: boolean;
  oldVersion: string | null; version: string; onBack: () => void; onInstall: () => void;
}) {
  return (
    <section className="panel panel--path">
      <p className="kicker reveal r1">第二事 · 择址</p>
      <h2 className="title reveal r2">安一处落脚</h2>

      <div className="mode reveal r3">
        {isUpdate ? (
          <span className="mode-pill mode-pill--up">
            原地更新<i>{oldVersion ? `v${oldVersion}` : "旧版"} → v{version}</i>
          </span>
        ) : (
          <span className="mode-pill">全新安装 · v{version}</span>
        )}
      </div>

      <label className="field reveal r3">
        <span className="field-cap">安装目录</span>
        <span className={`field-box ${isUpdate ? "is-locked" : ""}`}>
          <i className="field-ico"><GlyphFolder /></i>
          <input
            className="field-input"
            type="text"
            spellCheck={false}
            value={dir}
            onChange={(e) => setDir(e.currentTarget.value)}
            placeholder="C:\Users\…\Programs\NovelGenerateTeam"
            readOnly={isUpdate}
          />
        </span>
        <span className="field-hint">
          {isUpdate ? "已锁定为现有安装路径，无需更改。" : "默认安装至当前用户目录，无需管理员权限。"}
        </span>
      </label>

      <div className="row row--split reveal r4">
        <button className="ink-btn ink-btn--ghost" onClick={onBack}>
          <GlyphArrow back /> 返回
        </button>
        <button className="ink-btn ink-btn--solid" onClick={onInstall}
          disabled={dir.trim().length === 0}>
          {isUpdate ? "更新" : "安装"} <GlyphArrow />
        </button>
      </div>
    </section>
  );
}

/* ---- a smoothly tweened integer (easeOutCubic) -------------------------- */
function useCountUp(value: number, ms = 420): number {
  const [n, setN] = useState(value);
  const from = useRef(value);
  const raf = useRef<number | null>(null);
  useEffect(() => {
    if (reduced()) { from.current = value; setN(value); return; }
    const a = from.current, b = value, t0 = performance.now();
    if (a === b) return;
    const tick = (now: number) => {
      const t = Math.min(1, (now - t0) / ms);
      const e = 1 - Math.pow(1 - t, 3);
      const v = a + (b - a) * e;
      from.current = v; setN(v);
      if (t < 1) raf.current = requestAnimationFrame(tick);
      else from.current = b;
    };
    raf.current = requestAnimationFrame(tick);
    return () => { if (raf.current != null) cancelAnimationFrame(raf.current); };
  }, [value, ms]);
  return n;
}

/* ---- Step 镌刻 / Installing ---------------------------------------------- */
function Installing({
  percent, message, error, onRetry, onBack,
}: {
  percent: number; message: string; error: string | null;
  onRetry: () => void; onBack: () => void;
}) {
  const shown = useCountUp(percent);
  if (error) {
    return (
      <section className="panel panel--install is-error">
        <div className="carve-wrap">
          <CarveSeal reveal={0.5} char="误" size={120} dim />
        </div>
        <div className="install-side">
          <p className="kicker">镌刻受阻</p>
          <h2 className="title">安装未能完成</h2>
          <p className="errline" title={error}>{error}</p>
          <div className="row row--split">
            <button className="ink-btn ink-btn--ghost" onClick={onBack}>
              <GlyphArrow back /> 返回
            </button>
            <button className="ink-btn ink-btn--solid" onClick={onRetry}>重新镌刻</button>
          </div>
        </div>
      </section>
    );
  }
  return (
    <section className="panel panel--install">
      <div className="carve-wrap">
        <CarveSeal reveal={percent / 100} char="創" size={138} />
      </div>
      <div className="install-side">
        <p className="kicker">第三事 · 镌刻</p>
        <div className="pct">
          <span className="pct-num">{Math.round(shown)}</span>
          <span className="pct-sign">%</span>
        </div>
        <p className="install-msg" aria-live="polite" key={message}>{message}</p>
        <p className="install-sub">正为你于宣纸上镌一方印，请稍候。</p>
      </div>
    </section>
  );
}

/* ---- Step 钤印 / Done ---------------------------------------------------- */
function Done({
  isUpdate, version, shortcutWarning, onLaunch, onClose,
}: {
  isUpdate: boolean; version: string; shortcutWarning: string | null;
  onLaunch: () => void; onClose: () => void;
}) {
  const [stamped, setStamped] = useState(reduced());
  useEffect(() => {
    if (reduced()) { setStamped(true); return; }
    const t = window.setTimeout(() => setStamped(true), 240);
    return () => window.clearTimeout(t);
  }, []);
  return (
    <section className="panel panel--done">
      <div className="carve-wrap carve-wrap--done">
        <CarveSeal reveal={1} char="創" size={132} stamped={stamped} />
      </div>
      <div className="install-side">
        <p className="kicker reveal r1">第四事 · 钤印</p>
        <h2 className="title reveal r2">{isUpdate ? "更新已成" : "印成，可启程"}</h2>
        <p className="dek reveal r3">墨 · 创作 v{version} 已落墨于此机。</p>
        {shortcutWarning && (
          <p className="warnline reveal r3">
            未能创建快捷方式，可直接从安装目录启动：<br />
            <code>{shortcutWarning}\NovelGenerateTeam.exe</code>
          </p>
        )}
        <div className="row row--split reveal r4">
          <button className="ink-btn ink-btn--ghost" onClick={onClose}>完成</button>
          <button className="ink-btn ink-btn--solid" onClick={onLaunch}>
            启动 墨 · 创作 <GlyphArrow />
          </button>
        </div>
      </div>
    </section>
  );
}
