import type { Draft, Option, Profile } from "./domain";

export interface UserIdentity {
  id: string;
  name: string;
  nickName?: string;
  avatarPng?: number[] | null;
  lastOrganization: string;
}
export interface AuthResult {
  status: "valid" | "missing" | "invalid" | "unverified";
  user: UserIdentity | null;
  message: string;
}
export interface HistoryItem {
  id: string;
  identifier: string;
  title: string;
  status: string;
  createdAt: string;
  updatedAt: string;
}
export interface HistoryPage {
  items: HistoryItem[];
  page: number;
  hasMore: boolean;
}
export type SelectionMemory = Pick<
  Draft,
  | "workitemTypeId"
  | "assignedTo"
  | "sprint"
  | "versions"
  | "kind"
  | "business"
  | "customFields"
>;
export interface WorkspaceCommands {
  check_auth: AuthResult;
  login: AuthResult;
  list_organizations: Option[];
  list_projects: Option[];
  select_project: Profile;
  load_selection_memory: Partial<SelectionMemory>;
  remember_selections: void;
  list_remote_history: HistoryPage;
}
