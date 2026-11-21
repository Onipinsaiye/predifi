import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";

function sha256(value: string | Buffer): string {
  return createHash("sha256").update(value).digest("hex");
}

function sourceRevision(): string {
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: path.resolve(process.cwd()),
      encoding: "utf8",
    }).trim();
  } catch {
    return "0".repeat(40);
  }
}

const frontendRoot = path.resolve(process.cwd());
const lockfilePath = path.join(frontendRoot, "pnpm-lock.yaml");
const lockfileHash = sha256(readFileSync(lockfilePath));
const revision = sourceRevision();

export const buildProvenance = {
  sourceRevision: revision,
  lockfileHash,
  artifactDigest: process.env.BUILD_ARTIFACT_DIGEST ?? sha256(`${revision}:${lockfileHash}`),
  buildId: sha256(`${revision}:${lockfileHash}`),
  toolchain: {
    node: process.version,
    packageManager: `pnpm@${process.env.npm_config_user_agent?.match(/pnpm\/([^\s]+)/)?.[1] ?? "unknown"}`,
  },
};
