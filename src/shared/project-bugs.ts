import type { Option } from "./domain";

export type ProjectBugSort = "created_desc" | "created_asc";

export interface ProjectBugFilters {
  keyword: string;
  statusIds: string[];
  assignedToIds: string[];
  creatorIds: string[];
  tagIds: string[];
  createdFrom: string;
  createdTo: string;
  sort: ProjectBugSort;
}

export interface ProjectBugQuery extends ProjectBugFilters {
  page: number;
  perPage: number;
}

export interface ProjectBugItem {
  id: string;
  identifier: string;
  title: string;
  status: Option | null;
  assignedTo: Option | null;
  creator: Option | null;
  labels: Option[];
  createdAt: string;
  updatedAt: string;
}

export interface ProjectBugPage {
  items: ProjectBugItem[];
  page: number;
  perPage: number;
  total: number | null;
  totalPages: number | null;
  hasMore: boolean;
}

export interface ProjectBugFacets {
  statuses: Option[];
  members: Option[];
  tags: Option[];
  warnings: string[];
}

export type ProjectBugContentFormat = "markdown" | "richtext" | "plain";

export interface ProjectBugCustomField {
  id: string;
  name: string;
  format: string;
  values: string[];
}

export interface ProjectBugAttachment {
  id: string;
  fileId: string;
  name: string;
  suffix: string;
  size: number;
  creator: Option | null;
  createdAt: string;
}

export interface ProjectBugInlineFile {
  fileId: string;
  name: string;
  suffix: string;
  size: number;
}

export interface ProjectBugComment {
  id: string;
  parentId: string;
  author: Option | null;
  content: string;
  format: ProjectBugContentFormat;
  createdAt: string;
  updatedAt: string;
  pinned: boolean;
}

export interface ProjectBugActivity {
  id: string;
  eventType: string;
  actionType: string;
  actor: Option | null;
  propertyName: string;
  oldValues: string[];
  newValues: string[];
  relatedResourceId: string;
  relatedResourceType: string;
  occurredAt: string;
}

export interface ProjectBugSectionWarning {
  section: "attachments" | "comments" | "activities";
  message: string;
}

export interface ProjectBugDetail extends ProjectBugItem {
  description: string;
  format: ProjectBugContentFormat;
  modifier: Option | null;
  verifier: Option | null;
  participants: Option[];
  trackers: Option[];
  sprint: Option | null;
  statusUpdatedAt: string;
  archived: boolean;
  customFields: ProjectBugCustomField[];
  inlineFiles: ProjectBugInlineFile[];
  attachments: ProjectBugAttachment[];
  comments: ProjectBugComment[];
  activities: ProjectBugActivity[];
  sectionWarnings: ProjectBugSectionWarning[];
}

export interface ProjectBugAttachmentPreview {
  kind: "image" | "text" | "system";
  name: string;
  mime: string;
  data: string;
}

export const projectBugDetailCacheTtl = 2 * 60 * 1000;

export function isProjectBugDetailCacheFresh(
  loadedAt: number,
  now = Date.now(),
): boolean {
  return now >= loadedAt && now - loadedAt <= projectBugDetailCacheTtl;
}

export function emptyProjectBugFilters(): ProjectBugFilters {
  return {
    keyword: "",
    statusIds: [],
    assignedToIds: [],
    creatorIds: [],
    tagIds: [],
    createdFrom: "",
    createdTo: "",
    sort: "created_desc",
  };
}

export function projectBugQuery(
  filters: ProjectBugFilters,
  page = 1,
): ProjectBugQuery {
  return { ...structuredClone(filters), page, perPage: 50 };
}

export const projectBugSortOptions: Option[] = [
  { id: "created_desc", name: "创建时间：最新优先" },
  { id: "created_asc", name: "创建时间：最早优先" },
];

export function projectBugDateError(filters: ProjectBugFilters): string {
  if (!!filters.createdFrom !== !!filters.createdTo)
    return "创建时间需要同时填写开始和结束日期";
  if (
    filters.createdFrom &&
    filters.createdTo &&
    filters.createdFrom > filters.createdTo
  )
    return "创建时间的开始日期不能晚于结束日期";
  return "";
}
