// Claude Code hook (.claude/settings.json, PostToolUse on Edit, Write and MultiEdit): formats the file just edited
// with the repo's own formatters, Prettier for its file types and rustfmt for Rust. Reads the hook's JSON from
// stdin. It never fails the turn: whatever goes wrong, it exits 0 and the full check reports the formatting later.

import { spawnSync } from 'node:child_process';
import { isAbsolute, relative, resolve } from 'node:path';
import { formatFile } from './format.ts';
import { ROOT } from './lib/repo.ts';

/** The repo-relative path of the edited file, or `null` when the input names none inside the repo. */
export function editedPath(hookInput: string): string | null {
  let input: { tool_input?: { file_path?: unknown } };
  try {
    input = JSON.parse(hookInput) as typeof input;
  } catch {
    return null;
  }
  const file = input.tool_input?.file_path;
  if (typeof file !== 'string' || !file) return null;
  const rel = relative(ROOT, resolve(ROOT, file));
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) return null;
  return rel.replaceAll('\\', '/');
}

async function main(): Promise<void> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
  const path = editedPath(Buffer.concat(chunks).toString('utf8'));
  if (!path) return;
  if (path.endsWith('.rs')) {
    // rustfmt finds rustfmt.toml (the edition) from the file's folder.
    spawnSync('rustfmt', [resolve(ROOT, path)], { cwd: ROOT, stdio: 'ignore', timeout: 10_000 });
    return;
  }
  await formatFile(path, true);
}

if (import.meta.main) {
  try {
    await main();
  } catch {
    // Formatting is a convenience here; the checks enforce it.
  }
  process.exitCode = 0;
}
