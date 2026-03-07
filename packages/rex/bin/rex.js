#!/usr/bin/env node
"use strict";

const { execFileSync } = require("child_process");
const path = require("path");

const PLATFORM_PACKAGES = {
  "darwin arm64": "@limlabs/rex-darwin-arm64",
  "darwin x64": "@limlabs/rex-darwin-x64",
  "linux x64": "@limlabs/rex-linux-x64",
  "linux arm64": "@limlabs/rex-linux-arm64",
};

function getBinaryPath() {
  // Allow override via env var
  if (process.env.REX_BINARY_PATH) {
    return process.env.REX_BINARY_PATH;
  }

  const key = `${process.platform} ${process.arch}`;
  const pkg = PLATFORM_PACKAGES[key];

  if (!pkg) {
    console.error(
      `Rex does not have a prebuilt binary for your platform: ${process.platform} ${process.arch}\n` +
        `Supported platforms: macOS (arm64, x64), Linux (arm64, x64)\n` +
        `You can build from source: https://github.com/limlabs/rex`
    );
    process.exit(1);
  }

  try {
    return path.join(
      path.dirname(require.resolve(`${pkg}/package.json`)),
      "bin",
      "rex"
    );
  } catch {
    console.error(
      `The platform-specific package ${pkg} is not installed.\n` +
        `This usually means your package manager did not install optional dependencies.\n\n` +
        `Try:\n` +
        `  npm install ${pkg}\n` +
        `  # or reinstall with: npm install @limlabs/rex\n`
    );
    process.exit(1);
  }
}

const binary = getBinaryPath();

try {
  execFileSync(binary, process.argv.slice(2), { stdio: "inherit" });
} catch (e) {
  if (e.status !== null) {
    process.exit(e.status);
  }
  throw e;
}
