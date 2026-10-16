import { describe, expect, it } from "vitest";
import {
  emptyProjectBugFilters,
  isProjectBugDetailCacheFresh,
  projectBugDetailCacheTtl,
  projectBugDateError,
  projectBugQuery,
} from "../src/shared/project-bugs";

describe("项目 Bug 查询状态", () => {
  it("每次建立独立筛选对象并固定服务端页大小", () => {
    const first = emptyProjectBugFilters();
    first.statusIds.push("todo");
    const second = emptyProjectBugFilters();
    expect(second.statusIds).toEqual([]);
    expect(projectBugQuery(first, 3)).toEqual({
      ...first,
      page: 3,
      perPage: 50,
    });
  });

  it("创建时间必须组成有效范围", () => {
    const filters = emptyProjectBugFilters();
    filters.createdFrom = "2026-10-16";
    expect(projectBugDateError(filters)).toContain("同时填写");
    filters.createdTo = "2026-10-15";
    expect(projectBugDateError(filters)).toContain("不能晚于");
    filters.createdTo = "2026-10-16";
    expect(projectBugDateError(filters)).toBe("");
  });

  it("详情缓存仅在两分钟内复用并拒绝未来时间", () => {
    const now = Date.parse("2026-10-16T12:00:00Z");
    expect(isProjectBugDetailCacheFresh(now, now)).toBe(true);
    expect(
      isProjectBugDetailCacheFresh(now - projectBugDetailCacheTtl, now),
    ).toBe(true);
    expect(
      isProjectBugDetailCacheFresh(now - projectBugDetailCacheTtl - 1, now),
    ).toBe(false);
    expect(isProjectBugDetailCacheFresh(now + 1, now)).toBe(false);
  });
});
