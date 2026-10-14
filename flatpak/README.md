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

Capture from a shortcut with:

```sh
flatpak run io.github.gutopardini.wayshot --capture
```

`WAYSHOT_CAPTURE=1` still starts the same capture-first flow. WayShot requests
the screenshot portal before building the editor, and Esc in the screenshot UI
exits without opening the window. These output commands skip the editor:

```sh
flatpak run io.github.gutopardini.wayshot -- --clipboard
flatpak run io.github.gutopardini.wayshot -- --path ~/Pictures --clipboard
flatpak run io.github.gutopardini.wayshot -- --fullscreen --clipboard
```

On compositors that do not implement interactive portal screenshots, WayShot
automatically requests a full-screen capture instead. A dismissal does not.

JPEG XL export uses the GNOME runtime's `libjxl` library when available; the
format is hidden automatically if the runtime does not provide a working
encoder.
