// Rule: Warden reads other processes' parents, start times and memory only; never their command line, environment,
// executable path or working folder (a command line can hold a token or a prompt). Enforced on the `sysinfo` calls
// that would read them: the refresh kinds that ask for them, the accessors that return them, and the calls that
// read everything at once.

import { matches, type Rule, type Violation } from './types.ts';

const READS_DETAILS =
  /\bwith_(?:cmd|environ|exe|cwd|root)\b|\.(?:cmd|environ|exe|cwd|root)\(\)|\b(?:Process)?RefreshKind::everything\b|\bSystem::new_all\b|\.refresh_all\(|\.refresh_processes\(/g;

/** Drops `//` comments so doc comments may name what is never read. Strings containing `//` are cut too, harmlessly. */
function stripLineComments(text: string): string {
  return text
    .split('\n')
    .map((line) => line.replace(/\/\/.*$/, ''))
    .join('\n');
}

export const processDetails: Rule = {
  name: 'process-details',
  why: "Warden reads other processes' parents, start times and memory only: never their command line, environment, executable path or working folder.",
  applies: (path) => path.startsWith('crates/') && path.endsWith('.rs'),
  check(file) {
    const out: Violation[] = [];
    for (const m of matches(stripLineComments(file.text), READS_DETAILS)) {
      out.push({
        rule: this.name,
        path: file.path,
        line: m.line,
        message: `reads more of a process than its parent, start time and memory ("${m.match}")`,
      });
    }
    return out;
  },
};
