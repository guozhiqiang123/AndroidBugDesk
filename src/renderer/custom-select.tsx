import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { Option } from "../shared/domain";

let scrollLocks = 0;
let restorePageScroll: (() => void) | undefined;

export function lockPageScroll() {
  if (scrollLocks++ === 0) {
    const body = document.body;
    const overflow = body.style.overflow;
    const paddingRight = body.style.paddingRight;
    const scrollbarWidth =
      window.innerWidth - document.documentElement.clientWidth;
    const padding = Number.parseFloat(getComputedStyle(body).paddingRight) || 0;
    body.style.overflow = "hidden";
    if (scrollbarWidth > 0)
      body.style.paddingRight = `${padding + scrollbarWidth}px`;
    restorePageScroll = () => {
      body.style.overflow = overflow;
      body.style.paddingRight = paddingRight;
    };
  }
  return () => {
    if (--scrollLocks === 0) {
      restorePageScroll?.();
      restorePageScroll = undefined;
    }
  };
}

export interface CustomSelectProps {
  value: string | string[];
  options: Option[];
  onChange: (value: string | string[]) => void;
  multiple?: boolean;
  autoSelectFirst?: boolean;
  disabled?: boolean;
  label?: string;
  emptyLabel?: string;
  className?: string;
  loading?: boolean;
  loadingLabel?: string;
  onOpen?: () => void;
  selectionIndicator?: "start" | "end";
  optionAction?: {
    label: string;
    onClick: (option: Option) => void;
    disabled?: (option: Option) => boolean;
  };
  footerAction?: {
    label: string;
    onClick: () => void;
    disabled?: boolean;
  };
}

function selectionScrollAnchor(trigger: HTMLElement) {
  const containers: Element[] = [];
  const page = document.scrollingElement;
  for (
    let parent = trigger.parentElement;
    parent;
    parent = parent.parentElement
  ) {
    if (
      parent !== page &&
      /^(auto|scroll|overlay)$/.test(getComputedStyle(parent).overflowY)
    )
      containers.push(parent);
  }
  if (page) containers.push(page);
  return { top: trigger.getBoundingClientRect().top, containers };
}

/** A platform independent, keyboard accessible select used throughout the app. */
export function CustomSelect({
  value,
  options,
  onChange,
  multiple = false,
  autoSelectFirst = true,
  disabled = false,
  label,
  emptyLabel = "未选择",
  className = "",
  loading = false,
  loadingLabel = "正在读取…",
  onOpen,
  selectionIndicator = "end",
  optionAction,
  footerAction,
}: CustomSelectProps) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [query, setQuery] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const selectionAnchor = useRef<ReturnType<
    typeof selectionScrollAnchor
  > | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const defaulted = useRef(false);
  const optionKey = options.map((option) => option.id).join("\u0000");
  const previousOptionKey = useRef(optionKey);
  const controlId = useId();
  const values = Array.isArray(value) ? value : value ? [value] : [];
  const selected = options.filter((option) => values.includes(option.id));
  const first = selected[0] ?? (autoSelectFirst ? options[0] : undefined);

  useLayoutEffect(() => {
    if (!open || disabled || !options.length) return;
    return lockPageScroll();
  }, [open, disabled, options.length]);

  useLayoutEffect(() => {
    const anchor = selectionAnchor.current;
    selectionAnchor.current = null;
    if (!anchor || !button.current) return;
    // Preserve the control's visual position when selecting an option inserts
    // or removes fields above it. Inner scroll areas absorb the change first.
    for (const container of anchor.containers) {
      const delta = button.current.getBoundingClientRect().top - anchor.top;
      if (Math.abs(delta) < 0.5) break;
      container.scrollTop += delta;
    }
  });

  useEffect(() => {
    if (disabled || !options.length) setOpen(false);
  }, [disabled, options.length]);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  // Controls can opt out when an empty selection is their intended default.
  useEffect(() => {
    if (previousOptionKey.current !== optionKey) {
      previousOptionKey.current = optionKey;
      defaulted.current = false;
    }
    if (values.length) {
      defaulted.current = true;
      return;
    }
    if (!autoSelectFirst || disabled || defaulted.current || !options.length)
      return;
    defaulted.current = true;
    onChange(multiple ? [options[0].id] : options[0].id);
  }, [
    autoSelectFirst,
    disabled,
    multiple,
    onChange,
    optionKey,
    options,
    values.length,
  ]);

  const choose = (index: number) => {
    if (loading) return;
    if (index === options.length && footerAction) {
      if (footerAction.disabled) return;
      setOpen(false);
      button.current?.focus({ preventScroll: true });
      footerAction.onClick();
      return;
    }
    const option = options[index];
    if (!option) return;
    if (button.current)
      selectionAnchor.current = selectionScrollAnchor(button.current);
    if (multiple) {
      const next = values.includes(option.id)
        ? values.filter((id) => id !== option.id)
        : [...values, option.id];
      onChange(next);
      setActive(index);
      button.current?.focus({ preventScroll: true });
      return;
    }
    setOpen(false);
    onChange(option.id);
    button.current?.focus({ preventScroll: true });
  };
  const showMenu = () => {
    if (!open) onOpen?.();
    setOpen(true);
  };
  const move = (delta: number) => {
    if (!options.length) return;
    const count = options.length + (footerAction ? 1 : 0);
    setActive((index) => (index + delta + count) % count);
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      showMenu();
      move(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      showMenu();
      move(-1);
    } else if (event.key === "Home") {
      event.preventDefault();
      setActive(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setActive(Math.max(options.length - (footerAction ? 0 : 1), 0));
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!open) showMenu();
      else choose(active);
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      setOpen(false);
    } else if (event.key === "Tab") {
      if (!optionAction && !footerAction) setOpen(false);
    } else if (event.key.length === 1 && /\S/.test(event.key)) {
      const next = `${query}${event.key}`.toLowerCase();
      setQuery(next);
      const found = options.findIndex((item) =>
        item.name.toLowerCase().startsWith(next),
      );
      if (found >= 0) setActive(found);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setQuery(""), 700);
    }
  };
  return (
    <div
      className={`custom-select ${className}`}
      ref={root}
      // Cancel the enclosing label's default activation of the trigger button.
      onClick={(event) => event.preventDefault()}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button
        ref={button}
        type="button"
        className="custom-select-trigger"
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-controls={open ? `${controlId}-options` : undefined}
        aria-disabled={disabled || !options.length}
        disabled={disabled || !options.length}
        onClick={() => {
          if (open) setOpen(false);
          else showMenu();
          setActive(
            Math.max(
              0,
              options.findIndex((item) => item.id === values[0]),
            ),
          );
        }}
        onKeyDown={onKeyDown}
      >
        <span className={!selected.length ? "custom-select-empty" : ""}>
          {!selected.length && values.length
            ? `当前值不可用：${values.join("、")}`
            : multiple
              ? selected.length
                ? selected.map((item) => item.name).join("、")
                : options.length
                  ? emptyLabel
                  : "暂无可选项"
              : (first?.name ?? (options.length ? emptyLabel : "暂无可选项"))}
        </span>
        <span aria-hidden="true" className="custom-select-chevron">
          ⌄
        </span>
      </button>
      {open && !disabled && !!options.length && (
        <div
          className="custom-select-menu"
          id={`${controlId}-options`}
          role="listbox"
          aria-multiselectable={multiple || undefined}
          onWheel={(event) => event.stopPropagation()}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setOpen(false);
              button.current?.focus({ preventScroll: true });
            }
          }}
        >
          {loading && (
            <div className="custom-select-loading" role="status">
              {loadingLabel}
            </div>
          )}
          {options.map((option, index) => {
            const selectedOption = values.includes(option.id);
            const item = (
              <button
                type="button"
                role="option"
                aria-selected={selectedOption}
                disabled={loading}
                className={`custom-select-option ${index === active ? "active" : ""} ${selectionIndicator === "start" ? "leading-indicator" : ""}`}
                key={option.id}
                onMouseEnter={() => setActive(index)}
                onMouseDown={(event) => {
                  if (event.button === 0) event.preventDefault();
                }}
                onClick={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  choose(index);
                }}
              >
                {selectionIndicator === "start" && (
                  <span className="custom-select-indicator" aria-hidden="true">
                    {selectedOption ? "✓" : ""}
                  </span>
                )}
                <span>
                  {option.name}
                  {(option as Option & { description?: string }).description
                    ? ` · ${(option as Option & { description?: string }).description}`
                    : ""}
                </span>
                {selectionIndicator === "end" && selectedOption && (
                  <span aria-hidden="true">✓</span>
                )}
              </button>
            );
            return optionAction ? (
              <div
                className="custom-select-row"
                key={option.id}
                data-option-id={option.id}
              >
                {item}
                <button
                  type="button"
                  className="custom-select-item-action"
                  aria-label={`${optionAction.label} ${option.name}`}
                  disabled={optionAction.disabled?.(option)}
                  onMouseDown={(event) => {
                    if (event.button === 0) event.preventDefault();
                  }}
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    setOpen(false);
                    button.current?.focus({ preventScroll: true });
                    optionAction.onClick(option);
                  }}
                >
                  {optionAction.label}
                </button>
              </div>
            ) : (
              item
            );
          })}
          {footerAction && (
            <button
              type="button"
              className={`custom-select-footer-action ${active === options.length ? "active" : ""}`}
              disabled={footerAction.disabled}
              onMouseEnter={() => setActive(options.length)}
              onMouseDown={(event) => {
                if (event.button === 0) event.preventDefault();
              }}
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                choose(options.length);
              }}
            >
              {footerAction.label}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
