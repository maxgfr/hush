/**
 * The files of a drop, as the page gets them: picked with an `<input>` (with
 * `webkitdirectory`, their folders too) or dropped, folders walked. Paths are
 * made valid for the core (`drop::valid_path`) and distinct. Kept apart from
 * the sealing code so the first screen can accept a drop without loading it.
 */
import { UNSAFE } from "../app/protocol";
import { hush } from "../core/wasm";

export interface DropEntry {
  file: File;
  /** Relative path in the drop (`album/2024/a.jpg`). */
  path: string;
}

export const MAX_TOTAL = 4 * 1024 ** 3;
export const MAX_FILES = 4096;

/** Makes paths valid and distinct (two dropped files may share a name). */
function finish(entries: DropEntry[]): DropEntry[] {
  const core = hush();
  const seen = new Set<string>();
  const out: DropEntry[] = [];
  for (const e of entries) {
    let path = e.path
      .split("/")
      .filter((c) => c && c !== "." && c !== "..")
      .join("/")
      .replace(/\\/g, "_");
    path = Array.from(path, (ch) => {
      const c = ch.codePointAt(0) ?? 0;
      // Control characters, and the zero-width and bidi ones the core refuses (they spoof names).
      return c < 0x20 || (c >= 0x7f && c <= 0x9f) || UNSAFE.has(c) ? "_" : ch;
    }).join("");
    if (!path || !core.dropValidPath(path)) path = e.file.name.replace(/[/\\]/g, "_") || "file";
    const dot = path.lastIndexOf(".");
    const [stem, ext] =
      dot > path.lastIndexOf("/") + 1 ? [path.slice(0, dot), path.slice(dot)] : [path, ""];
    for (let n = 2; seen.has(path.toLowerCase()); n++) path = `${stem} (${n})${ext}`;
    seen.add(path.toLowerCase());
    out.push({ file: e.file, path });
  }
  return out;
}

/** Files picked with an `<input type=file>` (with `webkitdirectory`, their folders too). */
export function entriesFromFiles(list: FileList | File[]): DropEntry[] {
  return finish(
    Array.from(list).map((file) => ({
      file,
      path: (file as File & { webkitRelativePath?: string }).webkitRelativePath || file.name,
    })),
  );
}

interface FsEntry {
  isFile: boolean;
  isDirectory: boolean;
  name: string;
  fullPath: string;
  file?(ok: (f: File) => void, ko: (e: unknown) => void): void;
  createReader?(): { readEntries(ok: (e: FsEntry[]) => void, ko: (e: unknown) => void): void };
}

async function walk(entry: FsEntry, out: DropEntry[]): Promise<void> {
  if (out.length > MAX_FILES) return;
  if (entry.isFile && entry.file) {
    const file = await new Promise<File>((ok, ko) => entry.file?.(ok, ko));
    out.push({ file, path: entry.fullPath.replace(/^\/+/, "") });
  } else if (entry.isDirectory && entry.createReader) {
    const reader = entry.createReader();
    const all: FsEntry[] = [];
    // readEntries returns a batch at a time, until an empty one.
    for (;;) {
      const batch = await new Promise<FsEntry[]>((ok, ko) => reader.readEntries(ok, ko));
      if (batch.length === 0) break;
      all.push(...batch);
    }
    all.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    for (const e of all) await walk(e, out);
  }
}

/** Files and folders dropped on the page. */
export async function entriesFromDrop(dt: DataTransfer): Promise<DropEntry[]> {
  const roots: FsEntry[] = [];
  for (const i of Array.from(dt.items)) {
    if (i.kind !== "file") continue;
    const e = (i as unknown as { webkitGetAsEntry?(): FsEntry | null }).webkitGetAsEntry?.();
    if (e) roots.push(e);
  }
  if (roots.length === 0) return entriesFromFiles(dt.files);
  const out: DropEntry[] = [];
  for (const r of roots) await walk(r, out);
  return finish(out);
}
