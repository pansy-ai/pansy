// Prints the Lucide icons the window uses as entries of ui/icons/index.ts:
//   node scripts/icons.mjs <lucide-react's dist/esm/icons dir> bot server …
import fs from "node:fs";

const [dir, ...names] = process.argv.slice(2);
if (!dir || names.length === 0) {
  console.error("usage: node scripts/icons.mjs <lucide-react/dist/esm/icons> <icon>…");
  process.exit(1);
}
const attrs = (o) => Object.entries(o).filter(([k]) => k !== "key").map(([k, v]) => `${k}="${v}"`).join(" ");
let out = "";
for (const name of names) {
  let mod = await import(`${dir}/${name}.mjs`);
  if (!mod.__iconData) {
    // An alias: the module re-exports another icon.
    const target = fs.readFileSync(`${dir}/${name}.mjs`, "utf8").match(/from '\.\/([a-z0-9-]+)\.mjs'/);
    if (!target) throw new Error(`no icon ${name}`);
    mod = await import(`${dir}/${target[1]}.mjs`);
  }
  const inner = mod.__iconData.node.map(([tag, a]) => `<${tag} ${attrs(a)}/>`).join("");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
  const key = /^[a-z]+$/.test(name) ? name : JSON.stringify(name);
  out += `  ${key}: ${JSON.stringify(svg)},\n`;
}
process.stdout.write(out);
