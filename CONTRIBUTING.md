# Building and testing

Clone the repository and build the plugin executable:

```sh
git clone https://github.com/danieljvdm/herdr-composer
cd herdr-composer
bash build-composer.sh
herdr plugin link "$PWD"
```

The build uses Cargo.lock and replaces `bin/herdr-composer` atomically, so an
open editor can keep running while you rebuild.

## Tests

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 tests/catalog_test.py
python3 tests/acceptance.py
python3 tests/composer_pty_test.py
python3 tests/import_test.py
python3 tests/sow_test.py
```

The Rust tests cover request resolution, catalogs, draft conflicts, image import,
layout, and graphics stream cleanup. The graphics test needs local Unix-socket
permission. Python tests use disposable Git repositories, fake external tools,
and a real PTY. They do not launch real agents or alter the desktop clipboard.

For changes to Herdr or Worktrunk integration, start a disposable named Herdr
session and pass its socket to the live test:

```sh
python3 tests/live_integration.py --socket /path/to/test-session/herdr.sock
```

This test starts real Codex agents and leaves its source repositories under the
system temporary directory for inspection. It exercises both providers,
checks dirty-checkout refusal and unmerged-branch retention, and removes the
task checkouts. Stop the named test session when finished.

## Code map

- `src/request.rs` and `src/catalog.rs` resolve the choices shared by the CLI and editor.
- `src/session.rs` records launch progress and owns provider preparation and cleanup.
- `src/storage.rs` protects private state and concurrent draft revisions.
- `src/editor.rs`, `src/app.rs`, and `src/ui.rs` handle terminal editing.
- `src/images.rs` and `src/graphics.rs` retain originals and render previews.

Keep provider commands as argv arrays. Task text belongs in the saved request,
never in a generated shell command. Preserve the provider receipt before the
next workspace mutation. A lost prompt response must remain Unknown; it must
not trigger an automatic resend.

The implementation history and acceptance contract are in [COMPOSER_PLAN.md](COMPOSER_PLAN.md).

## Releases

Version each plugin independently. Bump the patch for fixes and the minor for
features or breaking changes while the plugin is pre-1.0. Keep the versions in
`herdr-plugin.toml`, `Cargo.toml`, and Composer's `Cargo.lock` entry equal.
Run the checks above, rebuild with `bash build-composer.sh`, and commit the release.
Tag that commit as `vX.Y.Z` and push the branch and tag together:

```sh
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push --atomic origin main vX.Y.Z
```

Install a tagged release with `herdr plugin install danieljvdm/herdr-composer --ref vX.Y.Z`.
Re-run install with the next tag to upgrade a GitHub-managed installation.
Local links use the working checkout and require a rebuild after source changes.
Never move a published release tag.
