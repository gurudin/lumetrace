import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

test("application and lockfile versions stay synchronized across npm, Rust, and Tauri", () => {
  const pkg = JSON.parse(read("package.json"));
  const npmLock = JSON.parse(read("package-lock.json"));
  const tauri = JSON.parse(read("src-tauri/tauri.conf.json"));
  const cargoPackage = read("src-tauri/Cargo.toml").split("[package]")[1]?.split("\n[")[0];
  const cargoLockPackage = read("src-tauri/Cargo.lock").split("[[package]]")
    .find((section) => /^name = "lumetrace"$/m.test(section));
  const versionOf = (section: string | undefined) => section?.match(/^version = "([^"]+)"$/m)?.[1];

  assert.match(pkg.version, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/);
  assert.equal(npmLock.version, pkg.version);
  assert.equal(npmLock.packages[""].version, pkg.version);
  assert.equal(tauri.version, pkg.version);
  assert.equal(versionOf(cargoPackage), pkg.version);
  assert.equal(versionOf(cargoLockPackage), pkg.version);
});
