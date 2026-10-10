import type { Field, Profile } from "./domain";

const excludedIds = new Set([
  "labels",
  "tags",
  "plannedStartDate",
  "plannedEndDate",
  "plannedStartTime",
  "plannedEndTime",
  "planStartTime",
  "planEndTime",
  "startDate",
  "endDate",
  "notFixReason",
  "nonFixReason",
  "estimatedTime",
]);

const excludedNames = new Set([
  "标签",
  "计划开始时间",
  "计划完成时间",
  "计划开始日期",
  "计划完成日期",
  "不修复理由",
  "不修复原因",
  "预计工时",
]);

export function excludedFieldId(id: string, profile: Profile): boolean {
  return (
    excludedIds.has(id) || excludedIds.has(profile.fieldMappings[id] ?? "")
  );
}

export function excludedField(field: Field, profile: Profile): boolean {
  return (
    excludedFieldId(field.id, profile) || excludedNames.has(field.name.trim())
  );
}

export function hiddenOptionalField(field: Field, profile: Profile): boolean {
  return (
    field.showWhenCreate === false &&
    !field.required &&
    !profile.requiredFieldIds.includes(field.id)
  );
}
