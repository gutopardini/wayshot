# Flatpak build

This project is intended to be Flatpak-first. The manifest lives at:
- `flatpak/io.github.gutopardini.wayshot.yml`

## One-time setup

Fedora:

```sh
sudo dnf install flatpak-builder python3
```

Ubuntu:

```sh
sudo apt update
sudo apt install flatpak-builder python3-venv
```

Make sure Flathub is available for the user installation used by the build:

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
```

## Generate cargo sources
Flatpak builds do not have network access by default, so Rust crates must be vendored via a generated sources file:

```sh
flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json
```

If `flatpak-cargo-generator` is not packaged by your distro, install it in a local virtualenv and run the same generator from there:

```sh
python3 -m venv .venv-flatpak-tools
.venv-flatpak-tools/bin/pip install flatpak-cargo-generator
.venv-flatpak-tools/bin/flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json
```

## Build + install

```sh
flatpak-builder --force-clean --install-deps-from=flathub --user --install build-flatpak flatpak/io.github.gutopardini.wayshot.yml
```

## GNOME shortcut

Use the environment variable for capture-first mode:

```sh
flatpak run --env=WAYSHOT_CAPTURE=1 io.github.gutopardini.wayshot
```

Flatpak can add a small cold-start delay before the screenshot portal appears.
On compositors that do not implement interactive portal screenshots, WayShot
automatically requests a full-screen capture instead.

JPEG XL export uses the GNOME runtime's `libjxl` library when available; the
format is hidden automatically if the runtime does not provide a working
encoder.
