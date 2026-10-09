import React, { useMemo, useState } from "react";
import type { UserIdentity } from "../shared/workspace";

export function userDisplayName(user: UserIdentity | null | undefined): string {
  return user?.nickName?.trim() || user?.name?.trim() || "云效用户";
}

export function UserProfile({
  user,
}: {
  user: UserIdentity | null | undefined;
}) {
  const name = userDisplayName(user);
  const source = useMemo(() => {
    if (!user?.avatarPng?.length) return null;
    return `data:image/png;base64,${btoa(user.avatarPng.map((byte) => String.fromCharCode(byte)).join(""))}`;
  }, [user?.avatarPng]);
  const [failedSource, setFailedSource] = useState<string | null>(null);
  const showImage = source && source !== failedSource;
  return (
    <div className="user-profile" aria-label="当前云效用户">
      <span className="user-avatar" aria-hidden="true">
        {showImage ? (
          <img src={source} alt="" onError={() => setFailedSource(source)} />
        ) : (
          Array.from(name)[0]
        )}
      </span>
      <div className="user-profile-text">
        <strong title={name}>{name}</strong>
        <span>
          <i className="dot" />
          云效账户
        </span>
      </div>
    </div>
  );
}
