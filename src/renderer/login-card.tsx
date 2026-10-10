import React, { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AuthResult } from "../shared/workspace";
import "./login.css";

export function LoginRestoring() {
  return (
    <div className="app login-page">
      <main>
        <section className="card login-card login-restoring" role="status">
          <div className="login-brand">
            <span className="login-mark" aria-hidden="true">
              A
            </span>
            <span>Android Bug Desk</span>
          </div>
          <span className="login-spinner" aria-hidden="true" />
          <p>正在恢复登录…</p>
        </section>
      </main>
    </div>
  );
}

export function LoginCard({
  token,
  setToken,
  remember,
  setRemember,
  error,
  busy,
  method,
  onPatLogin,
  onAuthorized,
}: {
  token: string;
  setToken: (value: string) => void;
  remember: boolean;
  setRemember: (value: boolean) => void;
  error: string;
  busy: boolean;
  method?: string;
  onPatLogin: () => void;
  onAuthorized: (result: AuthResult) => Promise<void>;
}) {
  const [mode, setMode] = useState(method === "pat" ? "pat" : "oauth");
  const [pending, setPending] = useState(false);
  const [oauthError, setOAuthError] = useState("");
  const attempt = useRef(0);
  const cancel = async () => {
    attempt.current += 1;
    await invoke("cancel_oauth_login").catch(() => undefined);
    setPending(false);
  };
  const authorize = async () => {
    const current = ++attempt.current;
    setPending(true);
    setOAuthError("");
    try {
      const result = await invoke<AuthResult>("oauth_login", { remember });
      if (current !== attempt.current) return;
      if (result.status === "valid") await onAuthorized(result);
      else setOAuthError(result.message);
    } catch (failure) {
      if (current === attempt.current) setOAuthError(String(failure));
    } finally {
      if (current === attempt.current) setPending(false);
    }
  };
  return (
    <section className="card login-card" aria-labelledby="login-title">
      <div className="login-brand">
        <span className="login-mark" aria-hidden="true">
          A
        </span>
        <span>Android Bug Desk</span>
      </div>
      <h2 id="login-title">登录云效</h2>
      <p className="login-intro">选择登录方式，进入缺陷工作台。</p>
      <div className="login-tabs" role="tablist" aria-label="登录方式">
        {[
          ["oauth", "授权登录"],
          ["pat", "填写 PAT"],
        ].map(([id, title]) => (
          <button
            key={id}
            type="button"
            role="tab"
            id={`login-tab-${id}`}
            aria-controls={`login-panel-${id}`}
            aria-selected={mode === id}
            disabled={busy}
            onClick={() => {
              if (pending) void cancel();
              setOAuthError("");
              setMode(id);
            }}
          >
            {title}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id={`login-panel-${mode}`}
        aria-labelledby={`login-tab-${mode}`}
      >
        {mode === "pat" ? (
          <form
            className="login-form"
            onSubmit={(event) => {
              event.preventDefault();
              onPatLogin();
            }}
          >
            <label className="field">
              <span>个人访问令牌</span>
              <input
                autoComplete="off"
                type="password"
                value={token}
                disabled={busy}
                onChange={(event) => setToken(event.target.value)}
                placeholder="输入你的个人访问令牌"
              />
              {error && (
                <small className="error-text" role="alert">
                  {error}
                </small>
              )}
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={remember}
                disabled={busy}
                onChange={(event) => setRemember(event.target.checked)}
              />
              记住登录
            </label>
            <button type="submit" className="primary" disabled={!token || busy}>
              确定
            </button>
          </form>
        ) : (
          <div className="login-form oauth-login">
            <p className="oauth-description">
              在系统浏览器中登录云效并确认授权，完成后自动返回客户端。
            </p>
            <label className="check">
              <input
                type="checkbox"
                checked={remember}
                disabled={busy || pending}
                onChange={(event) => setRemember(event.target.checked)}
              />
              记住登录
            </label>
            <button
              type="button"
              className="primary"
              disabled={busy || pending}
              onClick={() => void authorize()}
            >
              {pending ? "等待浏览器授权…" : "使用云效账号授权"}
            </button>
            {pending && (
              <button
                type="button"
                className="oauth-cancel"
                onClick={() => void cancel()}
              >
                取消授权
              </button>
            )}
            {(oauthError || error) && (
              <small className="error-text" role="alert">
                {oauthError || error}
              </small>
            )}
          </div>
        )}
      </div>
      <p className="login-note">
        {remember
          ? "登录凭据保存在系统安全存储中，下次启动自动恢复"
          : "登录凭据仅在本次会话中使用"}
      </p>
    </section>
  );
}
