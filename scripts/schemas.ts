import { z } from "zod";
import { writeFileSync } from "node:fs";
import { format } from "prettier";
import { draftSchema, emptyDraft, profileSchema } from "../src/shared/domain";
for (const [name, value] of Object.entries({
  draft: z.toJSONSchema(draftSchema),
  profile: z.toJSONSchema(profileSchema),
  "empty-draft": emptyDraft(
    "00000000-0000-4000-8000-000000000001",
    "2026-10-09T00:00:00Z",
  ),
})) {
  writeFileSync(
    `schemas/${name}${name === "empty-draft" ? "" : ".schema"}.json`,
    await format(JSON.stringify(value), { parser: "json" }),
  );
}
