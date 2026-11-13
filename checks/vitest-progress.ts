import type { Reporter, TestCase } from 'vitest/node';

/** Verbose reports completed tests; this also names a test that never completes. */
export default class TestProgress implements Reporter {
  onTestCaseReady(test: TestCase): void {
    console.log(`RUNNING ${test.module.moduleId}: ${test.fullName}`);
  }

  onTestCaseResult(test: TestCase): void {
    console.log(`FINISHED ${test.module.moduleId}: ${test.fullName}`);
  }
}
