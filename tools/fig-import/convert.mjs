// One-time import of withfig/autocomplete specs into flick's Spec format (ADR 0003, 0004).
// Usage: node convert.mjs <fig-src-dir> <node_modules-with-esbuild-and-fig-deps> <out-dir>
// Keeps Fig's structure as plain data. Anything that needs JavaScript is dropped, and
// a Generator that can't run without it is marked `needsCode: true`, which means it
// falls back to Native completion.
import { createRequire } from "node:module";
import { mkdtempSync, mkdirSync, readdirSync, writeFileSync, rmSync } from "node:fs";
import { join, relative, dirname } from "node:path";
import { tmpdir } from "node:os";
import { pathToFileURL } from "node:url";

const [figSrc, nodeModules, outDir] = process.argv.slice(2);
if (!outDir) throw new Error("usage: node convert.mjs <fig-src> <node_modules> <out-dir>");
const esbuild = createRequire(join(nodeModules, "x.js"))("esbuild");

const files = readdirSync(figSrc, { recursive: true }).filter((f) => f.endsWith(".ts"));
const bundleDir = mkdtempSync(join(tmpdir(), "fig-bundle-"));
esbuild.buildSync({
  entryPoints: files.map((f) => join(figSrc, f)),
  outbase: figSrc,
  outdir: bundleDir,
  outExtension: { ".js": ".mjs" },
  bundle: true,
  format: "esm",
  platform: "node",
  banner: { js: 'import { createRequire } from "node:module"; const require = createRequire(import.meta.url);' },
  nodePaths: [nodeModules],
  logLevel: "error",
});

const stats = { specs: 0, skipped: [], droppedFunctions: 0, generators: 0, needsCode: 0 };

const codeSamples = new Map();
const GENERATOR_KEEP = ["script", "splitOn", "template", "cache", "scriptTimeout", "trigger", "getQueryTerm"];
function generator(g) {
  stats.generators++;
  const needsCode =
    typeof g.script === "function" || typeof g.postProcess === "function" || typeof g.custom === "function" ||
    (!g.script && !g.template);
  const out = {};
  for (const k of GENERATOR_KEEP) if (g[k] !== undefined && typeof g[k] !== "function") out[k] = clean(g[k]);
  if (typeof g.script === "function" || typeof g.custom === "function") delete out.script;
  if (needsCode && process.env.FIG_DUMP_CODE) {
    const src = (g.postProcess ?? g.custom ?? g.script)?.toString?.() ?? "(no script)";
    codeSamples.set(src, (codeSamples.get(src) ?? 0) + 1);
  }
  if (needsCode) {
    stats.needsCode++;
    out.needsCode = true;
  }
  return out;
}

function clean(v) {
  if (typeof v === "function") {
    stats.droppedFunctions++;
    return undefined;
  }
  if (v instanceof RegExp) return v.source;
  if (Array.isArray(v)) return v.map(clean).filter((x) => x !== undefined);
  if (v === null || typeof v !== "object") return v;
  const out = {};
  for (const [k, x] of Object.entries(v)) {
    if (k === "generators") {
      out.generators = (Array.isArray(x) ? x : [x]).filter(Boolean).map(generator);
    } else if (k === "generateSpec" && typeof x === "function") {
      stats.droppedFunctions++;
      out.needsCode = true;
    } else {
      const c = clean(x);
      if (c !== undefined) out[k] = c;
    }
  }
  return out;
}

async function load(rel) {
  const mod = await import(pathToFileURL(join(bundleDir, rel.replace(/\.ts$/, ".mjs"))).href);
  let spec = mod.default;
  if (typeof spec === "function") spec = await spec(); // versioned spec: take latest
  if (spec?.versionedSpecPath) return load(spec.versionedSpecPath + ".ts");
  return spec;
}

rmSync(outDir, { recursive: true, force: true });
for (const rel of files.sort()) {
  let spec;
  try {
    spec = await load(rel);
  } catch (e) {
    stats.skipped.push(`${rel}: ${e.message.split("\n")[0]}`);
    continue;
  }
  if (!spec || typeof spec !== "object" || !spec.name) continue; // helper module, not a spec
  const dest = join(outDir, rel.replace(/\.ts$/, ".json"));
  mkdirSync(dirname(dest), { recursive: true });
  writeFileSync(dest, JSON.stringify(clean(spec)));
  stats.specs++;
}
rmSync(bundleDir, { recursive: true, force: true });

console.log(JSON.stringify({ ...stats, skipped: stats.skipped.length }, null, 2));
if (stats.skipped.length) console.log("skipped:\n  " + stats.skipped.join("\n  "));
if (process.env.FIG_DUMP_CODE)
  writeFileSync(process.env.FIG_DUMP_CODE, JSON.stringify([...codeSamples].sort((a, b) => b[1] - a[1])));
console.log(`wrote ${relative(process.cwd(), outDir) || outDir}`);
