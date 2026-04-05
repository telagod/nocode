#!/usr/bin/env node

"use strict";

// Verify the correct platform package was installed.
// If not, print a helpful message instead of failing silently at runtime.

const os = require("os");

const PLATFORMS = {
  "linux-x64": "@telagod/nocode-linux-x64",
  "darwin-x64": "@telagod/nocode-darwin-x64",
  "darwin-arm64": "@telagod/nocode-darwin-arm64",
};

const key = `${os.platform()}-${os.arch()}`;
const pkg = PLATFORMS[key];

if (!pkg) {
  console.warn(
    `[nocode] Warning: no prebuilt binary for ${key}. ` +
      `You may need to build from source.`
  );
  process.exit(0);
}

try {
  require.resolve(`${pkg}/package.json`);
} catch {
  console.warn(
    `[nocode] Warning: platform package ${pkg} was not installed. ` +
      `The 'nocode' command may not work. Try reinstalling.`
  );
}
