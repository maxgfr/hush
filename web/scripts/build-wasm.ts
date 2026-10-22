/**
 * Builds the Rust core for the browser:
 *   cargo build -p hush-wasm --profile wasm --target wasm32-unknown-unknown
 *   → wasm-bindgen --target web (its version must match the crate's, checked below)
 *   → wasm-opt -Oz (binaryen, pinned in bun.lock)
 * Output: src/core/pkg/ (git-ignored), imported by src/core/wasm.ts.
 */
import { mkdir, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";

const WEB = resolve(import.meta.dir, "..");
const ROOT = resolve(WEB, "..");
const OUT = join(WEB, "src/core/pkg");

async function run(cmd: string[], cwd = ROOT): Promise<string> {
  const proc = Bun.spawn(cmd, { cwd, stdout: "pipe", stderr: "inherit" });
  const out = await new Response(proc.stdout).text();
  if ((await proc.exited) !== 0) throw new Error(`build-wasm: ${cmd.join(" ")} failed`);
  return out;
}

/** The wasm-bindgen version locked in Cargo.lock: the CLI must be the exact same. */
export async function lockedBindgenVersion(): Promise<string> {
  const lock = await readFile(join(ROOT, "Cargo.lock"), "utf8");
  const m = lock.match(/name = "wasm-bindgen"\nversion = "([^"]+)"/);
  if (!m?.[1]) throw new Error("build-wasm: wasm-bindgen not found in Cargo.lock");
  return m[1];
}

export async function buildWasm(): Promise<void> {
  const want = await lockedBindgenVersion();
  const have = (await run(["wasm-bindgen", "--version"])).trim().split(" ")[1];
  if (have !== want) {
    throw new Error(
      `build-wasm: wasm-bindgen CLI ${have} does not match the crate (${want}); run \`cargo install wasm-bindgen-cli --version ${want} --locked\``,
    );
  }
  await run([
    "cargo",
    "build",
    "--locked",
    "-p",
    "hush-wasm",
    "--profile",
    "wasm",
    "--target",
    "wasm32-unknown-unknown",
  ]);
  await rm(OUT, { recursive: true, force: true });
  await mkdir(OUT, { recursive: true });
  await run([
    "wasm-bindgen",
    "--target",
    "web",
    "--out-dir",
    OUT,
    join(ROOT, "target/wasm32-unknown-unknown/wasm/hush_wasm.wasm"),
  ]);
  await run([
    join(WEB, "node_modules/.bin/wasm-opt"),
    "-Oz",
    // Exactly what rustc's default wasm32 target emits; `--all-features` would let binaryen use
    // proposals browsers do not ship yet.
    "--enable-bulk-memory",
    "--enable-mutable-globals",
    "--enable-nontrapping-float-to-int",
    "--enable-sign-ext",
    "--enable-reference-types",
    "--enable-multivalue",
    "-o",
    join(OUT, "hush_wasm_bg.wasm"),
    join(OUT, "hush_wasm_bg.wasm"),
  ]);
}

if (import.meta.main) {
  const started = performance.now();
  await buildWasm();
  const size = (await Bun.file(join(OUT, "hush_wasm_bg.wasm")).arrayBuffer()).byteLength;
  console.log(
    `wasm: ${(size / 1024).toFixed(0)} KiB in ${Math.round(performance.now() - started)} ms`,
  );
}
