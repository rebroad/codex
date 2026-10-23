/** Resolve the native Codex package target for a Node.js host. */
export function codexTargetTriple(
  platform: NodeJS.Platform,
  arch: string,
  prefix?: string,
): string | null {
  const isTermux =
    platform === "android" || (platform === "linux" && prefix?.includes("/com.termux/"));

  switch (platform) {
    case "linux":
    case "android":
      switch (arch) {
        case "x64":
          return "x86_64-unknown-linux-musl";
        case "arm64":
          return isTermux ? "aarch64-linux-android" : "aarch64-unknown-linux-musl";
        case "arm":
          return "armv7-unknown-linux-musleabihf";
        default:
          return null;
      }
    case "darwin":
      switch (arch) {
        case "x64":
          return "x86_64-apple-darwin";
        case "arm64":
          return "aarch64-apple-darwin";
        default:
          return null;
      }
    case "win32":
      switch (arch) {
        case "x64":
          return "x86_64-pc-windows-msvc";
        case "arm64":
          return "aarch64-pc-windows-msvc";
        default:
          return null;
      }
    default:
      return null;
  }
}
