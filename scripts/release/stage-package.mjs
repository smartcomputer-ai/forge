#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const [kind, directory, version, gitSha] = process.argv.slice(2);
if (!kind || !directory || !version || !gitSha) {
  throw new Error("usage: stage-package.mjs <client|configurator> <directory> <version> <git-sha>");
}

const packagePath = path.join(directory, "package.json");
const manifest = JSON.parse(fs.readFileSync(packagePath, "utf8"));
manifest.version = version;
manifest.private = false;

if (kind === "client") {
  manifest.publishConfig = { access: "public" };
  fs.writeFileSync(
    path.join(directory, "release.json"),
    `${JSON.stringify({ version, gitSha }, null, 2)}\n`,
  );
} else if (kind === "configurator") {
  manifest.private = true;
  manifest.dependencies["@lightspeed/agent-client"] = "file:./agent-client.tgz";
  delete manifest.scripts?.prepare;
} else {
  throw new Error(`unknown package kind: ${kind}`);
}

fs.writeFileSync(packagePath, `${JSON.stringify(manifest, null, 2)}\n`);
const lockPath = path.join(directory, "package-lock.json");
if (fs.existsSync(lockPath)) {
  fs.rmSync(lockPath);
}
