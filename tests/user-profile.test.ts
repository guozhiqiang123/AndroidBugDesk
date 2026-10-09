import { describe, expect, it } from "vitest";
import { userDisplayName } from "../src/renderer/user-profile";

const user = { id: "user-a", name: "企业姓名", lastOrganization: "org-a" };
describe("当前用户展示名", () => {
  it("优先个人昵称，再用显示名称，空响应不展示内部ID", () => {
    expect(userDisplayName({ ...user, nickName: "  测试昵称  " })).toBe(
      "测试昵称",
    );
    expect(userDisplayName({ ...user, nickName: " " })).toBe("企业姓名");
    expect(userDisplayName({ ...user, name: "" })).toBe("云效用户");
    expect(userDisplayName(null)).toBe("云效用户");
  });
});
