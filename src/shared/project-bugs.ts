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
