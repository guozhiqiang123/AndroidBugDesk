import type { ShortcutPresentation } from "../shared/shortcuts";

export function ShortcutBadge({
  shortcut,
}: {
  shortcut: ShortcutPresentation;
}) {
  return (
    <kbd className="shortcut-hint" aria-hidden="true">
      {shortcut.display}
    </kbd>
  );
}
