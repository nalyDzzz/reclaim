// Writes the latest.json manifest that Reclaim's built-in updater reads.
//
// usage: node update-manifest.mjs <dir> <version> <download-base-url> [notes-file]
//
// <dir> holds the installers and the .sig files from `tauri signer sign`.
// Installed copies look up "windows-x86_64-nsis" or "-msi" to match how
// they were installed; anything else falls back to "windows-x86_64".
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, version, baseUrl, notesFile] = process.argv.slice(2);
if (!dir || !version || !baseUrl) {
  console.error("usage: node update-manifest.mjs <dir> <version> <download-base-url> [notes-file]");
  process.exit(2);
}

const files = readdirSync(dir);

function entry(suffix) {
  const file = files.find((name) => name.endsWith(suffix));
  if (!file) throw new Error(`No file ending in ${suffix} in ${dir}`);
  const sigPath = join(dir, `${file}.sig`);
  if (!existsSync(sigPath)) throw new Error(`${file} has no .sig file next to it`);
  return {
    url: `${baseUrl.replace(/\/$/, "")}/${encodeURIComponent(file)}`,
    signature: readFileSync(sigPath, "utf8").trim(),
  };
}

const nsis = entry("-setup.exe");
const msi = entry(".msi");
const notes = notesFile && existsSync(notesFile) ? readFileSync(notesFile, "utf8").trim() : "";

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64-nsis": nsis,
    "windows-x86_64-msi": msi,
    "windows-x86_64": nsis,
  },
};

writeFileSync(join(dir, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Wrote ${join(dir, "latest.json")} for ${version}`);
