#!/usr/bin/env node
"use strict";

const { execFileSync, execSync } = require("child_process");
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

// ANSI helpers (matching Rust display helpers)
const bold = (s) => `\x1b[1m${s}\x1b[0m`;
const dim = (s) => `\x1b[2m${s}\x1b[0m`;
const greenBold = (s) => `\x1b[1;32m${s}\x1b[0m`;

function isRexGloballyInstalled() {
  try {
    execSync("rex --version", { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

const binary = getBinaryPath();
const args = process.argv.slice(2);
const isInit = args[0] === "init";

try {
  execFileSync(binary, args, { stdio: "inherit" });
} catch (e) {
  if (e.status !== null) {
    process.exit(e.status);
  }
  throw e;
}

// Post-init: install dependencies, ensure rex is globally available, print instructions
if (isInit && args.length >= 2) {
  const projectName = args[1];
  const projectDir = path.resolve(projectName);

  // Install project dependencies (react, react-dom)
  process.stderr.write(`  Installing dependencies...\n\n`);
  try {
    execSync("npm install --silent", {
      cwd: projectDir,
      stdio: ["ignore", "ignore", "inherit"],
    });
    process.stderr.write(
      `  ${greenBold("✓")} ${greenBold("Dependencies installed")}\n\n`
    );
  } catch {
    process.stderr.write(
      `  ${dim("Could not install dependencies. Run 'npm install' in the project directory.")}\n\n`
    );
  }

  // Install rex globally if not already available
  if (!isRexGloballyInstalled()) {
    process.stderr.write(`  Installing rex globally...\n\n`);
    try {
      execSync("npm install -g @limlabs/rex", {
        stdio: ["ignore", "ignore", "inherit"],
      });
      process.stderr.write(
        `  ${greenBold("✓")} ${greenBold("Rex installed globally")}\n\n`
      );
    } catch {
      process.stderr.write(
        `  ${dim("Could not install rex globally. Run 'npm install -g @limlabs/rex' manually.")}\n\n`
      );
    }
  }

  // Print get started instructions (zero-config: no npm commands)
  process.stderr.write(`  ${dim("Get started:")}\n\n`);
  process.stderr.write(`    ${bold("cd")} ${bold(projectName)}\n`);
  process.stderr.write(`    ${bold("rex dev")}\n\n`);
}
