use crate::model::{Field, Profile};

const EXCLUDED_IDS: &[&str] = &[
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
];

const EXCLUDED_NAMES: &[&str] = &[
    "标签",
    "计划开始时间",
    "计划完成时间",
    "计划开始日期",
    "计划完成日期",
    "不修复理由",
    "不修复原因",
    "预计工时",
];

pub fn excluded_field(f: &Field, p: &Profile) -> bool {
    excluded_id(&f.id)
        || p.field_mappings
            .get(&f.id)
            .is_some_and(|target| excluded_id(target))
        || EXCLUDED_NAMES.contains(&f.name.trim())
}

pub fn excluded_id(id: &str) -> bool {
    EXCLUDED_IDS.contains(&id)
}

pub fn hidden_optional_field(f: &Field, p: &Profile) -> bool {
    !f.show_when_create && !f.required && !p.required_field_ids.contains(&f.id)
}
