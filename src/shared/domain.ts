import { z } from "zod";

const text = z.string().max(50_000);
export const idSchema = z.string().regex(/^[A-Za-z0-9_-]{1,128}$/);
export const profileSchema = z.object({
  name: z.string().min(1).max(100),
  edition: z.literal("central"),
  organizationId: idSchema,
  projectId: idSchema,
  requiredFieldIds: z.array(idSchema).default([]),
  fieldMappings: z
    .record(
      idSchema,
      z.enum([
        "subject",
        "description",
        "assignedTo",
        "sprint",
        "versions",
        "verifier",
        "participants",
        "labels",
        "trackers",
      ]),
    )
    .default({}),
  enableVersions: z.boolean().default(true),
  enableSprints: z.boolean().default(true),
  attachmentWarningMb: z.number().positive().default(50),
});
export type Profile = z.infer<typeof profileSchema>;
export const sourceSchema = z.object({
  status: z.enum(["collected", "failed", "manual", "not_applicable"]),
  at: text,
  detail: text,
});
export type Source = z.infer<typeof sourceSchema>;
export const attachmentSchema = z.object({
  id: z.uuid(),
  name: text,
  path: text,
  mime: text,
  size: z.number().nonnegative(),
  sha256: z.string().regex(/^[a-f0-9]{64}$/),
  role: z.enum(["log", "image", "video", "report", "sample"]),
  source: text,
  coverageStart: text.default(""),
  coverageEnd: text.default(""),
  confirmed: z.boolean(),
  derivedFrom: z.string().nullable().default(null),
});
export type Attachment = z.infer<typeof attachmentSchema>;
export const markdownAssetSchema = z.strictObject({
  id: z.uuid(),
  field: z.enum(["actual", "expected", "preconditions", "steps"]),
  name: text,
  path: text,
  mime: z.enum(["image/png", "image/jpeg"]),
  size: z.number().nonnegative(),
  sha256: z.string().regex(/^[a-f0-9]{64}$/),
  width: z.number().int().positive(),
  height: z.number().int().positive(),
  state: z.enum([
    "staged",
    "unreferenced",
    "local",
    "uploading",
    "uploaded",
    "failed",
  ]),
  remoteId: z.string().nullable(),
  embedMarkdown: text,
  embedUrl: text,
  error: text,
});
export type MarkdownAsset = z.infer<typeof markdownAssetSchema>;
export const draftSchema = z.strictObject({
  id: z.uuid(),
  sessionId: z.uuid().nullable(),
  jenkinsPackageUrl: text,
  title: text,
  bugPage: text.optional(),
  kind: z.enum([
    "功能",
    "UI",
    "多语言",
    "动态过程",
    "崩溃",
    "ANR",
    "广告",
    "性能",
    "网络",
    "数据",
  ]),
  workitemTypeId: text,
  assignedTo: text,
  sprint: text,
  versions: z.array(text),
  app: z.strictObject({
    packageName: text,
    versionName: text,
    versionCode: text,
  }),
  device: z.object({
    model: text,
    androidVersion: text,
    totalMemKb: text,
    availableMemKb: text,
    resolution: text,
    density: text,
  }),
  event: z.strictObject({
    occurredAt: text,
    originalAt: text,
    source: z.enum(["auto", "manual"]),
    sourceNote: text,
  }),
  preconditions: text,
  steps: text,
  expected: text,
  actual: text,
  frequency: z.object({
    firstDiscovery: z.boolean(),
    attempts: text,
    occurrences: text,
  }),
  network: z.object({
    operation: text,
    type: text,
    error: text,
    collectionStatus: text,
  }),
  data: z.object({ type: text, count: text, source: text }),
  customFields: z.record(idSchema, z.union([text, z.array(text)])),
  sources: z.record(z.string(), sourceSchema),
  collectionErrors: z.array(text),
  attachments: z.array(attachmentSchema),
  markdownAssets: z.array(markdownAssetSchema),
  previewConfirmed: z.boolean(),
  createdAt: text,
  updatedAt: text,
});
export type Draft = z.infer<typeof draftSchema>;
export interface Option {
  id: string;
  name: string;
}
export interface Field {
  id: string;
  name: string;
  format: string;
  type: string;
  required: boolean;
  showWhenCreate?: boolean;
  optionError?: string;
  options: Option[];
}
export interface Metadata {
  profile: Profile;
  typeId: string;
  types: Option[];
  fields: Field[];
  members: Option[];
  versions: Option[];
  sprints: Option[];
  fetchedAt: string;
}
export interface ValidationIssue {
  field: string;
  message: string;
}
export type SubmissionState =
  | "draft"
  | "ready"
  | "creating"
  | "create_unknown"
  | "created"
  | "uploading"
  | "finalizing"
  | "partial"
  | "submitted";
export interface UploadRecord {
  localId: string;
  name: string;
  sha256: string;
  remoteId?: string;
  state: "pending" | "uploading" | "unknown" | "confirmed";
  error?: string;
}
export interface Evidence {
  id: string;
  directory: string;
  zipPath: string;
  sha256: string;
  files: Attachment[];
}
export interface Submission {
  draftId: string;
  profile: Profile;
  state: SubmissionState;
  evidence: Evidence;
  workitemId?: string;
  identifier?: string;
  url?: string;
  uploads: UploadRecord[];
  markdownAssets?: MarkdownAsset[];
  error?: string;
  intendedAt: string;
  updatedAt: string;
}
export interface Device {
  serial: string;
  state: string;
  label: string;
}
export interface Snapshot {
  device: Draft["device"];
  app: Partial<Draft["app"]>;
  sources: Record<string, Source>;
  errors: string[];
  details: Record<string, unknown>;
  deviceTime: string | null;
  capturedAt: string;
}
export interface Session {
  id: string;
  serial: string;
  packageName: string;
  startedAt: string;
  stoppedAt: string | null;
  snapshot: Snapshot;
  logFiles: string[];
  gaps: string[];
}
export function emptyDraft(id: string, now = new Date().toISOString()): Draft {
  return {
    id,
    sessionId: null,
    jenkinsPackageUrl: "",
    title: "",
    bugPage: "",
    kind: "功能",
    workitemTypeId: "",
    assignedTo: "",
    sprint: "",
    versions: [],
    app: {
      packageName: "",
      versionName: "",
      versionCode: "",
    },
    device: {
      model: "",
      androidVersion: "",
      totalMemKb: "",
      availableMemKb: "",
      resolution: "",
      density: "",
    },
    event: {
      occurredAt: "",
      originalAt: "",
      source: "manual",
      sourceNote: "",
    },
    preconditions: "",
    steps: "",
    expected: "",
    actual: "",
    frequency: { firstDiscovery: true, attempts: "", occurrences: "" },
    network: { operation: "", type: "", error: "", collectionStatus: "" },
    data: { type: "", count: "", source: "" },
    customFields: {},
    sources: {},
    collectionErrors: [],
    attachments: [],
    markdownAssets: [],
    previewConfirmed: false,
    createdAt: now,
    updatedAt: now,
  };
}
