# Project Workflow & Constraints

## Pre-Push Verification
Always ensure the following checks pass before concluding a task or pushing changes. If these tools are not available in the current environment, perform a manual review of the diff to ensure compliance.

- **Formatting:** `cargo fmt --all -- --check`
  - Avoid trailing whitespaces.
  - Keep lines within reasonable lengths (GTK/Cairo calls can get long).
- **Linting:** `cargo clippy --all-targets -- -D warnings`
  - Avoid collapsible `if` statements (use `and_then` or `let` chains).
  - Ensure `Cargo.lock` is synced with `Cargo.toml` (run `cargo check` after version bumps).
- **Build:** `cargo build`
- **Tests:** `cargo test`

## Flatpak Releases
- When updating the version, always sync `Cargo.toml`, `Cargo.lock`, and `metainfo.xml`.
- Update the Flatpak manifest with the exact commit hash and tag.
- Ensure the commit hash in the Flathub repository manifest matches the final commit hash of the tag in the main repository.
