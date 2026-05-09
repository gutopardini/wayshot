# WayShot

WayShot is a fast screenshot and annotation tool for Linux, built around a clean GNOME/Wayland workflow.

It is designed for the common “capture, mark, copy, paste” loop:

1. Trigger a screenshot.
2. Annotate it quickly.
3. Press `Ctrl+C`.
4. Paste the final image into chat, issues, docs, or email.

WayShot is based on [GreatShot](https://github.com/furqanalishah/greatshot), with a renamed identity, visual refresh, GNOME capture helper, clipboard workflow improvements, and additional editor changes.

## Features

- Screenshot capture through the XDG desktop portal.
- Capture-first helper command for GNOME/Wayland: `wayshot-gnome-capture`.
- Open images from disk.
- Paste images from the clipboard.
- Annotate with pen, rectangle, circle/ellipse, line, arrow, text, and blur/pixelate.
- Select and move annotations.
- Copy the rendered result to the clipboard.
- `Ctrl+C` copies through `wl-copy` and closes WayShot when the copy succeeds.
- Save the final image as PNG.
- Glass-style editor UI with image backdrop.

## Install

Fedora dependencies:

```sh
sudo dnf install rust cargo gtk4-devel libadwaita-devel pkgconf-pkg-config gcc wl-clipboard
```

Build and install locally:

```sh
make install
```

This installs:

```text
~/.local/bin/wayshot
~/.local/bin/wayshot-gnome-capture
~/.local/share/applications/io.github.gutopardini.wayshot.desktop
~/.local/share/icons/hicolor/scalable/apps/io.github.gutopardini.wayshot.svg
```

Make sure `~/.local/bin` is in your `PATH`.

## Usage

Capture and open the editor:

```sh
wayshot-gnome-capture
```

Open an existing image:

```sh
wayshot path/to/image.png
```

Run the app normally:

```sh
wayshot
```

Recommended GNOME shortcut:

```text
Command: wayshot-gnome-capture
Shortcut: choose your preferred keybinding
```

## Editor Workflow

- Use the floating toolbar to choose an annotation tool.
- Use the color palette to change annotation color.
- Use settings to change stroke size, zoom, capture delay, and interactive capture.
- Press `Ctrl+C` to copy the rendered image and close the window.
- Use the copy button if you want to copy without closing the app.
- Use save to export a PNG.

## Build

Run from source:

```sh
cargo run
```

Build release:

```sh
cargo build --release --locked
```

Validate formatting and compile tests:

```sh
cargo fmt -- --check
cargo test
```

## Flatpak

The Flatpak manifest lives at:

```text
flatpak/io.github.gutopardini.wayshot.yml
```

Generate vendored Cargo sources before building a Flatpak:

```sh
flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json
```

Build and install locally:

```sh
flatpak-builder --force-clean --install-deps-from=flathub --user --install build-flatpak flatpak/io.github.gutopardini.wayshot.yml
```

## Credits

WayShot is based on [GreatShot](https://github.com/furqanalishah/greatshot).

GreatShot was created by Furqan Ali Shah as a minimal screenshot and annotation tool for GNOME/Wayland. WayShot keeps that foundation and evolves it with a new identity, workflow, and visual direction.
