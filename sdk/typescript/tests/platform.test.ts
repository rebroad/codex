import { describe, expect, it } from "@jest/globals";

import { codexTargetTriple } from "../src/platform";

describe("codexTargetTriple", () => {
  it("selects the Android package for Android arm64", () => {
    expect(codexTargetTriple("android", "arm64")).toBe("aarch64-linux-android");
  });

  it("selects the Android package for Termux reported as Linux", () => {
    expect(codexTargetTriple("linux", "arm64", "/data/data/com.termux/files/usr")).toBe(
      "aarch64-linux-android",
    );
  });

  it("keeps regular Linux arm64 on the musl package", () => {
    expect(codexTargetTriple("linux", "arm64", "/usr")).toBe("aarch64-unknown-linux-musl");
  });
});
