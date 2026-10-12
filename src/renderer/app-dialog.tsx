import {
  useEffect,
  useId,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { lockPageScroll } from "./custom-select";
import "./app-dialog.css";

export function AppDialog({
  title,
  closeLabel,
  onClose,
  returnFocusRef,
  className = "",
  children,
}: {
  title: string;
  closeLabel?: string;
  onClose: () => void;
  returnFocusRef?: RefObject<HTMLElement | null>;
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = ref.current!;
    const opener =
      returnFocusRef?.current ?? (document.activeElement as HTMLElement | null);
    const unlock = lockPageScroll();
    dialog.showModal();
    return () => {
      if (dialog.open) dialog.close();
      unlock();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
      window.requestAnimationFrame(() => {
        if (opener?.isConnected) opener.focus({ preventScroll: true });
      });
    };
  }, []);

  return createPortal(
    <dialog
      ref={ref}
      className={`app-dialog ${className}`.trim()}
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onKeyDown={(event) => {
        if (event.key !== "Tab") return;
        const controls = Array.from(
          event.currentTarget.querySelectorAll<HTMLElement>(
            "button,input,textarea,select,a[href],[tabindex]",
          ),
        ).filter(
          (element) =>
            !element.matches(":disabled") &&
            element.tabIndex >= 0 &&
            element.getClientRects().length,
        );
        const first = controls[0];
        const last = controls.at(-1);
        if (
          (event.shiftKey && document.activeElement === first) ||
          (!event.shiftKey && document.activeElement === last)
        ) {
          event.preventDefault();
          (event.shiftKey ? last : first)?.focus();
        }
      }}
    >
      <div className="app-dialog-titlebar">
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="app-dialog-close"
          aria-label={closeLabel ?? `关闭${title}`}
          onClick={onClose}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </div>
      <div className="app-dialog-scroll">{children}</div>
    </dialog>,
    document.body,
  );
}
