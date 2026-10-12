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
  | "customFields"
>;
export interface ProjectSetting {
  projectId: string;
  enabled: boolean;
  packageName: string;
}
export interface ProjectSettings {
  entries: ProjectSetting[];
}
export interface DraftProjectScope {
  organizationId: string;
  projectId: string;
}
export interface WorkspaceCommands {
  check_auth: AuthResult;
  login: AuthResult;
  oauth_login: AuthResult;
  cancel_oauth_login: void;
  list_organizations: Option[];
  list_projects: Option[];
  select_project: Profile;
  load_project_settings: ProjectSettings;
  save_project_settings: ProjectSettings;
  load_selection_memory: Partial<SelectionMemory>;
  remember_selections: void;
  list_remote_history: HistoryPage;
}
