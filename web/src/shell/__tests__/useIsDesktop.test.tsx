import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { useIsDesktop } from "../useIsDesktop";

function ServerSnapshot() {
  return <span>{String(useIsDesktop())}</span>;
}

describe("useIsDesktop server snapshot", () => {
  it("uses the mobile layout while rendering on the server", () => {
    expect(renderToString(<ServerSnapshot />)).toBe("<span>false</span>");
  });
});
