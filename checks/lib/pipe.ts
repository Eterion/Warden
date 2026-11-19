// Sends made-up hook events to a running core over its per-user pipe, as `warden event` does
// (`warden_core::pipe`): the account and the event cut down to the fields the hook keeps, as one JSON line, then the
// core's one-byte answer. For the demo and the budgets, which have no CLI to run the hooks.

import { readFileSync } from 'node:fs';
import { connect } from 'node:net';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

/** The fields `warden event` keeps (`warden_core::pipe::HookEvent`); the core refuses any other. */
const KEPT = [
  'hook_event_name',
  'session_id',
  'cwd',
  'transcript_path',
  'tool_name',
  'agent_id',
  'agent_type',
  'notification_type',
];

/** Sends one event for `account` to the core whose data folder is `data`. */
export function pipeSend(data: string, account: string, event: Record<string, unknown>): Promise<void> {
  const { pipe } = JSON.parse(readFileSync(join(data, 'pipe.json'), 'utf8')) as { pipe: string };
  const kept = Object.fromEntries(Object.entries(event).filter(([k]) => KEPT.includes(k)));
  return new Promise((resolve, reject) => {
    const socket = connect(pipe);
    socket.once('error', reject);
    socket.once('data', () => {
      socket.end();
      resolve();
    });
    socket.write(JSON.stringify({ account, event: kept }) + '\n');
  });
}

/** Waits until the core has written its pipe's name into the data folder. */
export async function pipeReady(data: string): Promise<void> {
  const deadline = performance.now() + 10_000;
  for (;;) {
    try {
      readFileSync(join(data, 'pipe.json'));
      return;
    } catch {
      if (performance.now() > deadline) throw new Error('Warden did not open its pipe');
      await sleep(20);
    }
  }
}
