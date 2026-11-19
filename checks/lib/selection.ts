/** Test paths, including Rust unit-test modules and UI fixtures. */
export function isTest(path: string): boolean {
  return /(?:^|\/)(?:tests?|__tests__|__snapshots__)(?:\/|\.rs$)/.test(path) || /\.(?:test|spec)\.[^.]+$/.test(path);
}

export function affectsBudgets(path: string): boolean {
  if (isTest(path)) return false;
  return (
    // The budget harness and every local helper it imports can change the measurement.
    ['checks/budgets.ts', 'checks/budgets.json', 'checks/lib/pipe.ts', 'checks/lib/repo.ts'].includes(path) ||
    /^crates\/[^/]+\/src\//.test(path) ||
    // The demo server the core budgets measure.
    path === 'crates/warden-server/examples/demo.rs' ||
    /(?:^|\/)Cargo\.(?:toml|lock)$/.test(path) ||
    path.startsWith('.cargo/') ||
    /^rust-toolchain(?:\.toml)?$/.test(path) ||
    /^crates\/[^/]+\/build\.rs$/.test(path) ||
    /^crates\/warden-desktop\/(?:tauri\..*\.json|capabilities\/|permissions\/|icons\/)/.test(path) ||
    /^ui\/(?:src\/|public\/|index\.html$|vite\.config\.|package\.json$|tsconfig)/.test(path) ||
    path === 'package.json' ||
    path === 'package-lock.json'
  );
}
