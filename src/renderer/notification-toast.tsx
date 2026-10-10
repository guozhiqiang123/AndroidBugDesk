import React, { useCallback, useEffect, useRef, useState } from "react";

export interface Notification {
  id: number | string;
  message: string;
  kind: "success" | "warning" | "error" | "loading";
}

export function useTransientNotice() {
  const [notice, updateNotice] = useState<Notification | null>(null);
  const sequence = useRef(0);
  const setNotice = useCallback(
    (message: string, kind: Notification["kind"] = "success") => {
      updateNotice(message ? { id: ++sequence.current, message, kind } : null);
    },
    [],
  );
  return { notice, setNotice };
}

export function NotificationToast({
  notification,
  onDismiss,
}: {
  notification: Notification | null;
  onDismiss: () => void;
}) {
  const dismiss = useRef(onDismiss);
  useEffect(() => {
    dismiss.current = onDismiss;
  }, [onDismiss]);
  useEffect(() => {
    if (!notification || notification.kind === "loading") return;
    const timer = window.setTimeout(
      () => dismiss.current(),
      notification.kind === "success" ? 3500 : 6000,
    );
    return () => window.clearTimeout(timer);
  }, [notification?.id, notification?.kind]);
  if (!notification) return null;
  return (
    <div
      className={`notification-toast ${notification.kind}`}
      role={notification.kind === "error" ? "alert" : "status"}
      aria-atomic="true"
    >
      <span className="notification-icon" aria-hidden="true">
        {notification.kind === "loading"
          ? ""
          : notification.kind === "success"
            ? "✓"
            : "!"}
      </span>
      <p>{notification.message}</p>
      {notification.kind !== "loading" && (
        <button type="button" aria-label="关闭消息提示" onClick={onDismiss}>
          ×
        </button>
      )}
    </div>
  );
}
