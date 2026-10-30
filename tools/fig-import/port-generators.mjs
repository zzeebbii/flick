// Applies ports.json to the converted Specs in place. Idempotent; run after editing ports.json.
// Usage: node tools/fig-import/port-generators.mjs [specs-dir]
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const specsDir = process.argv[2] ?? join(here, "../../specs");
const ports = JSON.parse(readFileSync(join(here, "ports.json"), "utf8"));
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

let total = 0;
for (const [cli, rules] of Object.entries(ports)) {
  if (cli.startsWith("_")) continue;
  const file = join(specsDir, `${cli}.json`);
  const spec = JSON.parse(readFileSync(file, "utf8"));
  const hits = new Array(rules.length).fill(0);
  const visit = (v) => {
    if (Array.isArray(v)) return v.forEach(visit);
    if (!v || typeof v !== "object") return;
    for (const g of Array.isArray(v.generators) ? v.generators : []) {
      rules.forEach((r, i) => {
        // Match on the original script; a port that rewrote `script` records it in `portedFrom`.
        const original = g.portedFrom ?? g.script;
        const match = r.custom ? g.ported || (!g.script && !g.template && g.needsCode) : same(original, r.script);
        if (!match) return;
        if (r.set.script && !g.portedFrom) g.portedFrom = g.script;
        Object.assign(g, r.set);
        delete g.needsCode;
        if (r.custom) g.ported = true;
        hits[i]++;
      });
    }
    Object.values(v).forEach(visit);
  };
  visit(spec);
  rules.forEach((r, i) => {
    if (!hits[i]) console.warn(`${cli}: rule ${i} matched nothing: ${JSON.stringify(r.script ?? "custom")}`);
  });
  writeFileSync(file, JSON.stringify(spec));
  const n = hits.reduce((a, b) => a + b, 0);
  total += n;
  console.log(`${cli}: ported ${n} Generators`);
}
console.log(`total: ${total}`);
