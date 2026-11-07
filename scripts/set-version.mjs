// Sets the version everywhere it lives: package.json, the Cargo workspace and
// tauri.conf.json (the one the installer and the updater read).
//   node scripts/set-version.mjs 0.2.0
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version ?? "")) {
  console.error("Give a version like 0.2.0");
  process.exit(1);
}
const root = fileURLToPath(new URL("..", import.meta.url));
const update = (file, change) => {
  const path = root + file;
  writeFileSync(path, change(readFileSync(path, "utf8")));
  console.log(`${file} → ${version}`);
};

update("package.json", (s) => s.replace(/"version": "[^"]+"/, `"version": "${version}"`));
update("src-tauri/tauri.conf.json", (s) => s.replace(/"version": "[^"]+"/, `"version": "${version}"`));
update("Cargo.toml", (s) => s.replace(/(\[workspace\.package\][^[]*?version = )"[^"]+"/, `$1"${version}"`));
console.log(`Now: commit, then  git tag v${version} && git push origin main v${version}`);
