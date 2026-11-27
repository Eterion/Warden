# Making a release

Releases are made only when the user asks. Each one is built, signed and published by GitHub Actions
(`.github/workflows/release.yml`) when a version tag is pushed; the installed app finds it within the hour, downloads it
and offers "Install and restart".

## Versions

Warden stays on 0 until the user says it has enough features and is stable: `0.MINOR.PATCH`, a minor step for new
features (0.1.0, 0.2.0), a patch for fixes (0.2.1). The version is the workspace's (`version` under `[workspace.package]`
in `Cargo.toml`); the app and its installer take it from there.

## Steps

1. Write the release's notes in `CHANGELOG.md`, under a new `## X.Y.Z` heading at the top, with `### New` and
   `### Fixes and improvements` lists (leave out an empty one). Keep them short and general, what is new and what was
   fixed, from the commits since the last tag (`git log --oneline <last tag>..master`). The workflow refuses a tag without
   them.
2. On `master`, with its checks passed: set the new version in `Cargo.toml` and run `cargo build` so `Cargo.lock`
   follows.
3. Commit both, the notes and the version: `chore: release 0.2.0`.
4. Tag and push both:

   ```sh
   git tag v0.2.0
   git push origin master v0.2.0
   ```

5. The workflow checks that the tag matches the version and takes the version's notes from `CHANGELOG.md` (failing
   when the section is missing or empty), builds the UI, `warden.exe` and the installer as
   `npm run desktop:release` does, signs the update, and publishes the release with the installer, its signature
   (`.sig`) and `latest.json`, the manifest the app reads at
   `https://github.com/Eterion/Warden/releases/latest/download/latest.json`.
6. Check the release on GitHub. An installed Warden finds it at its next hourly look, or when it starts.

## Release notes

A version's section of `CHANGELOG.md` (without its heading; `node checks/release-notes.ts X.Y.Z <file>` prints it) is
published as the GitHub release's notes, under the title "Warden X.Y.Z", and as `notes` in `latest.json`.

## The update signing key

Updates are signed with the updater's own key (Tauri's signer), separate from Windows code signing (the installer is
not Windows code-signed, so SmartScreen warns once on a fresh install).

- The public key is `plugins.updater.pubkey` in `crates/warden-desktop/tauri.conf.json`: every installed Warden checks
  each update against it before running anything.
- The private key and its password are the repository's secrets `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and one offline backup the user keeps. Neither is ever written into the
  repository, a log or a message.
- A local `npm run desktop:installer` needs no key: only the release build (`desktop:release`) makes the signed update
  artifacts.

If the private key is lost, installed copies cannot update any more: make a new key, put its public half in
`tauri.conf.json` and the private half in the secrets, release, and install that release by hand once.
