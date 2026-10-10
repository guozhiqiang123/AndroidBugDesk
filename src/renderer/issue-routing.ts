import type { Attachment, Draft, ValidationIssue } from "../shared/domain";

export type IssueRoute = Record<string, string[]>;

export interface IssueNavigationGroup {
  target: string;
  field: string;
  messages: string[];
}

/** Keeps one summary entry per control, even when native and cloud fields share it. */
export function groupIssuesByNavigationTarget(
  issues: ValidationIssue[],
  targetFor: (field: string) => string,
): IssueNavigationGroup[] {
  const groups = new Map<string, IssueNavigationGroup>();
  for (const issue of issues) {
    const target = targetFor(issue.field);
    const group = groups.get(target);
    if (!group) {
      groups.set(target, {
        target,
        field: issue.field,
        messages: [issue.message],
      });
      continue;
    }
    if (
      group.field.startsWith("customFields.") &&
      !issue.field.startsWith("customFields.")
    )
      group.field = issue.field;
    if (!group.messages.includes(issue.message))
      group.messages.push(issue.message);
  }
  return [...groups.values()];
}

/** Keeps attachment validation visible at the control which can fix it. */
export function routeIssues(
  draft: Draft,
  issues: ValidationIssue[],
): IssueRoute {
  const routes: IssueRoute = {};
  const push = (target: string, message: string) => {
    const messages = (routes[target] ??= []);
    if (!messages.includes(message)) messages.push(message);
  };
  for (const issue of issues) {
    if (issue.field !== "attachments") {
      push(issue.field, issue.message);
      continue;
    }
    const message = issue.message;
    if (message === "派生副本必须携带全部来源回执") {
      let located = false;
      for (const log of draft.attachments.filter((a) => a.role === "log")) {
        const paired = draft.attachments.filter(
          (a) => a.role === "report" && a.derivedFrom === log.id,
        );
        const window = log.source.startsWith("ADB 问题日志窗口派生副本");
        const redacted = log.source.startsWith("文本派生副本");
        const rangeRequired =
          window ||
          log.source.includes("来源为问题日志窗口") ||
          (redacted &&
            draft.attachments.some(
              (a) =>
                a.id === log.derivedFrom &&
                a.source.startsWith("ADB 问题日志窗口派生副本"),
            ));
        if (
          (window || redacted || paired.length) &&
          ((rangeRequired &&
            !paired.some((a) => a.source === "日志窗口范围与缺口记录")) ||
            (redacted &&
              !paired.some((a) => a.source === "文本替换记录（请核对副本）")))
        ) {
          located = true;
          push(attachmentIssueKey(log, "confirmed"), message);
        }
      }
      if (!located) push("attachment-import", message);
    } else if (message === "来源回执必须保留关联副本") {
      let located = false;
      for (const receipt of draft.attachments.filter(
        (a) => a.role === "report" && a.derivedFrom,
      )) {
        if (!draft.attachments.some((a) => a.id === receipt.derivedFrom)) {
          located = true;
          push(attachmentIssueKey(receipt, "confirmed"), message);
        }
      }
      if (!located) push("attachment-import", message);
    } else {
      push("attachment-import", message);
    }
  }
  return routes;
}

export function attachmentIssueKey(
  attachment: Attachment,
  suffix: "coverageStart" | "coverageEnd" | "confirmed",
) {
  return `attachment.${attachment.id}.${suffix}`;
}
