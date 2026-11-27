// A release's notes: its section of CHANGELOG.md, which the release workflow publishes on the GitHub release and in
// latest.json (docs/releasing.md).
//
//   node checks/release-notes.ts <X.Y.Z> <out-file>   writes the section's text without a trailing
//                                                    newline, or fails when it is missing, empty or only headings

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { ROOT } from './lib/repo.ts';

/**
 * The text under `## <version>` in a changelog, up to the next `## ` heading, without the heading itself and trimmed.
 * Throws when the section is missing, empty or holds only headings.
 */
export function releaseNotes(changelog: string, version: string): string {
  const lines = changelog.split(/\r?\n/);
  const start = lines.findIndex((line) => line.trimEnd() === `## ${version}`);
  if (start < 0) throw new Error(`CHANGELOG.md has no "## ${version}" section: write the release's notes there first.`);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((line) => line.startsWith('## '));
  const notes = (end < 0 ? rest : rest.slice(0, end)).join('\n').trim();
  if (!notes)
    throw new Error(`CHANGELOG.md's "## ${version}" section is empty: write the release's notes there first.`);
  if (!notes.split('\n').some((line) => line.trim() && !line.trimStart().startsWith('#')))
    throw new Error(
      `CHANGELOG.md's "## ${version}" section has only headings: write the release's notes under them first.`,
    );
  return notes;
}

function main(): void {
  const [version, out] = process.argv.slice(2);
  if (!version || !out) {
    console.error('usage: node checks/release-notes.ts <X.Y.Z> <out-file>');
    process.exit(2);
  }
  try {
    const notes = releaseNotes(readFileSync(join(ROOT, 'CHANGELOG.md'), 'utf8'), version);
    // As written, no trailing newline: the GitHub release and latest.json carry exactly this text.
    writeFileSync(out, notes);
    console.log(`release notes for ${version}:\n${notes}`);
  } catch (error) {
    console.error((error as Error).message);
    process.exit(1);
  }
}

if (import.meta.main) main();
