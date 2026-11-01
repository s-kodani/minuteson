// Run after npm ci: node app/scripts/generate-windows-unplated-icons.mjs
// Commit the PNGs; production builds only stage them and do not render icons.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const app = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const icons = join(app, "src-tauri", "icons");
const source = join(icons, "icon.svg");
const sizes = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256];
const forms = ["", "_altform-unplated", "_altform-lightunplated"];
const scratchRoot = resolve(app, "..", "output");
mkdirSync(scratchRoot, { recursive: true });
const scratch = mkdtempSync(join(scratchRoot, "windows-icon-render-"));
try {
  const args = [join(app, "node_modules", "@tauri-apps", "cli", "tauri.js"),
    "icon", source, "--output", scratch, ...sizes.flatMap(size => ["--png", String(size)])];
  const result = spawnSync(process.execPath, args, { cwd: app, stdio: "inherit" });
  if (result.error || result.status !== 0) throw result.error ?? Error(`Icon generation failed: ${result.status}`);
  for (const size of sizes) {
    const png = readFileSync(join(scratch, `${size}x${size}.png`));
    if (png.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a"
      || png.readUInt32BE(16) !== size || png.readUInt32BE(20) !== size
      || png[24] !== 8 || png[25] !== 6 || png[28] !== 0) {
      throw Error(`Expected a non-interlaced ${size}x${size} RGBA PNG`);
    }
    // The existing dark rounded tile and white mark are readable on either
    // Windows theme, so all forms retain the same brand artwork and alpha.
    for (const form of forms) writeFileSync(join(icons, `Square44x44Logo.targetsize-${size}${form}.png`), png);
  }
  console.log(`Generated ${sizes.length * forms.length} Windows target-size icons from icon.svg`);
  console.log(`SVG SHA-256: ${createHash("sha256").update(readFileSync(source)).digest("hex")}`);
} finally {
  // Only remove the new scratch directory created directly under output.
  if (dirname(resolve(scratch)) !== scratchRoot) throw Error("Unexpected icon scratch directory");
  rmSync(scratch, { recursive: true, force: true });
}
