# Flatpak build

This project is intended to be Flatpak-first. The manifest lives at:
- `flatpak/io.github.gutopardini.wayshot.yml`

## One-time setup (Fedora)
- `sudo dnf install flatpak-builder flatpak-builder-tools`

## Generate cargo sources
Flatpak builds do not have network access by default, so Rust crates must be vendored via a generated sources file:
- `flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json`

## Build + install
- `flatpak-builder --force-clean --install-deps-from=flathub --user --install build-flatpak flatpak/io.github.gutopardini.wayshot.yml`

## Submission source
The manifest references the GitHub repo:
- `https://github.com/gutopardini/wayshot`
