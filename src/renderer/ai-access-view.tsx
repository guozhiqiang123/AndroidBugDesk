import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AiIntegrationInfo } from "../shared/workspace";

interface AiAccessViewProps {
  enabled: boolean;
  projectReady: boolean;
  busy: boolean;
  onToggle: (enabled: boolean) => Promise<void>;
  notify: (message: string, tone?: "success" | "warning" | "error") => void;
}

export function AiAccessView({
  enabled,
  projectReady,
  busy,
  onToggle,
  notify,
}: AiAccessViewProps) {
  const [info, setInfo] = useState<AiIntegrationInfo | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [copying, setCopying] = useState(false);

  const load = async () => {
    setLoading(true);
    setError("");
    try {
      setInfo(await invoke<AiIntegrationInfo>("prepare_ai_integration"));
    } catch (cause) {
      setError(typeof cause === "string" ? cause : String(cause));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const copyPrompt = async () => {
    setCopying(true);
    setError("");
    try {
      const prepared = await invoke<AiIntegrationInfo>(
        "copy_ai_install_prompt",
      );
      setInfo(prepared);
      notify("已复制安装提示语，请直接粘贴到 Codex。", "success");
    } catch (cause) {
      setError(typeof cause === "string" ? cause : String(cause));
    } finally {
      setCopying(false);
    }
  };

  return (
    <div className="ai-access-page">
      <header className="ai-access-header">
        <div>
          <span className="eyebrow">CODEX · MCP</span>
          <h1>接入 AI</h1>
          <p>
            让 Codex 通过本机受限工具填写
            Bug、采集证据和准备提交预览。云效凭据始终留在客户端，最终提交仍由使用者确认。
          </p>
        </div>
        <span className={`ai-access-state ${enabled ? "ready" : "off"}`}>
          {enabled ? "本机接入已开启" : "本机接入未开启"}
        </span>
      </header>

      <section className="card ai-readiness-card">
        <div className="card-title">
          <div>
            <h2>接入准备</h2>
            <p className="muted-text">
              安装只需执行一次；使用时请保持 Android Bug Desk 正在运行。
            </p>
          </div>
          <label className="switch-control">
            <input
              type="checkbox"
              role="switch"
              aria-label="允许本机 AI 接入"
              checked={enabled}
              disabled={busy}
              onChange={(event) => void onToggle(event.target.checked)}
            />
            <span className="switch-track" aria-hidden="true">
              <span className="switch-thumb" />
            </span>
            <strong>{enabled ? "AI 接入已开启" : "允许本机 AI 接入"}</strong>
          </label>
        </div>
        <div className="ai-readiness-grid">
          <div className="ai-readiness-item ready">
            <span aria-hidden="true">✓</span>
            <div>
              <strong>客户端已登录</strong>
              <small>Codex 不会接触云效登录凭据</small>
            </div>
          </div>
          <div
            className={`ai-readiness-item ${projectReady ? "ready" : "pending"}`}
          >
            <span aria-hidden="true">{projectReady ? "✓" : "!"}</span>
            <div>
              <strong>{projectReady ? "项目已就绪" : "项目尚未配置"}</strong>
              <small>
                {projectReady
                  ? "AI 只操作当前项目"
                  : "请先在设置中启用并选择项目"}
              </small>
            </div>
          </div>
          <div className={`ai-readiness-item ${info ? "ready" : "pending"}`}>
            <span aria-hidden="true">{info ? "✓" : "…"}</span>
            <div>
              <strong>
                {info ? "内置接入包已就绪" : "正在准备内置接入包"}
              </strong>
              <small>Skill 与说明来自当前客户端版本</small>
            </div>
          </div>
        </div>
      </section>

      <section className="card ai-install-card">
        <div className="card-title">
          <div>
            <h2>复制给 Codex</h2>
            <p className="muted-text">
              把下面整段提示语复制到 Codex，Codex 会安装内置 Skill、登记 MCP
              并核对连接。
            </p>
          </div>
          <div className="actions ai-install-actions">
            <button
              type="button"
              onClick={() => void load()}
              disabled={loading || copying}
            >
              刷新路径
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void copyPrompt()}
              disabled={!info || loading || copying}
            >
              {copying ? "复制中…" : "复制给 Codex"}
            </button>
          </div>
        </div>
        {loading && (
          <p className="muted-text" role="status">
            正在准备内置接入资源…
          </p>
        )}
        {error && (
          <p className="error-text" role="alert">
            {error}
          </p>
        )}
        {info && (
          <>
            <pre className="ai-install-prompt" aria-label="Codex 安装提示语">
              {info.installPrompt}
            </pre>
            <dl className="ai-integration-paths">
              <div>
                <dt>客户端程序</dt>
                <dd>{info.executablePath}</dd>
              </div>
              <div>
                <dt>内置 Skill</dt>
                <dd>{info.skillPath}</dd>
              </div>
              <div>
                <dt>MCP 服务</dt>
                <dd>
                  {info.mcpServerName} · {info.platform} · v{info.bundleVersion}
                </dd>
              </div>
            </dl>
          </>
        )}
      </section>

      <section className="card ai-usage-card">
        <h2>接入后怎么用</h2>
        <ol>
          <li>保持客户端运行、登录并选中要提交 Bug 的项目。</li>
          <li>
            在 Codex 中描述问题，并明确要求使用 Android Bug Desk 准备 Bug。
          </li>
          <li>Codex 会填草稿、按需采集或导入附件，并把提交预览交回客户端。</li>
          <li>在客户端核对负责人、验证者、证据和正文后点击提交。</li>
        </ol>
        <p className="ai-example-prompt">
          示例：往当前项目提一个
          Bug，把这段崩溃信息写入问题表现，采集崩溃日志，负责人和验证者都是郭志强。准备好后让我在客户端确认提交。
        </p>
      </section>
    </div>
  );
}
