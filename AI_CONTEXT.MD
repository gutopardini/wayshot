# AI Context

This repository contains **WayShot**, a minimal Linux screenshot and annotation tool focused on GNOME/Wayland workflows.

## Project Shape

- Language: Rust, edition 2024.
- UI stack: GTK 4 via `gtk4`, libadwaita via `libadwaita`, Cairo drawing through GTK.
- Screenshot backend: XDG desktop portal through `ashpd`, with a full-screen
  fallback when the current compositor does not support interactive capture.
- Async runtime: `tokio`, used for portal screenshot requests and delay handling.
- Packaging: Flatpak-first manifest under `flatpak/`, plus local desktop install through `Makefile`.
- App ID: `io.github.gutopardini.wayshot`.

## Key Files

- `src/main.rs`: app entry point. Parses CLI options, starts a portal capture before GTK when one was requested, then either writes the headless output or creates an `adw::Application` and delegates UI construction to `ui::build_ui`.
- `src/cli.rs`: argument parsing for capture mode, delay, and headless output.
- `src/capture.rs`: portal screenshot requests. Interactive dismissal, including GNOME's Escape response, does not fall back to a full-screen capture.
- `src/output.rs`: headless save, clipboard copy, and stdout output.
- `src/ui.rs`: most application wiring. Builds the GTK/libadwaita window, header actions, floating tool palette, color palette, settings popover, screenshot portal flow, file open/save, clipboard copy/paste, zoom, and input event controllers.
- `src/editor.rs`: editor model and drawing engine. Defines `EditorState`, `Annotation`, `Tool`, coordinate transforms, hit testing, movement, rendering to pixbuf, and Cairo drawing for annotations.
- `assets/icons/`: SVG icons embedded at compile time with `include_bytes!`.
- `data/io.github.gutopardini.wayshot.desktop`: desktop entry.
- `data/io.github.gutopardini.wayshot.metainfo.xml`: AppStream metadata.
- `flatpak/io.github.gutopardini.wayshot.yml`: Flatpak build manifest.
- `flatpak/cargo-sources.json`: generated vendored crate source list for Flatpak builds.
- `Makefile`: local `make install` target.
- `scripts/wayshot-gnome-capture`: helper script that launches `wayshot --capture`, which starts hidden, requests a screenshot through the portal, then presents itself with the captured image loaded.

## Runtime Behavior

The application opens a resizable libadwaita window with a drawing area and floating palettes.

Image sources:

- Capture screenshot through the portal (`ashpd::desktop::screenshot::Screenshot`).
- Open an image from disk through `gtk::FileDialog`.
- Paste an image from the clipboard.
- Launch with a file path argument, for example `wayshot screenshot.png`.
- `wayshot --capture`, `-c`, or `WAYSHOT_CAPTURE=1` starts a capture and then opens the editor. The portal request is issued before the editor UI is built.
- `--fullscreen` and `--region` choose the portal mode. The default still tries interactive capture and falls back to full screen only when the portal cannot offer it.
- `--path DIR`, `--clipboard`, and `--raw` finish without opening the editor. `--path` and `--clipboard` can be combined. `--format` overrides the output format for that run. `--delay SECONDS` waits before the portal request.
- Pressing Esc in the GNOME screenshot UI cancels the capture. A shortcut launch exits without presenting WayShot. GNOME reports that dismissal as portal response 2, not response 1.
- Use `wayshot-gnome-capture` after `make install` to start WayShot in capture-first mode. It executes `wayshot --capture`.

Outputs:

- Copy rendered screenshot plus annotations to clipboard.
- Press `Ctrl+C` to copy the rendered image to clipboard.
- Save the rendered image in a format supported by a working runtime encoder.
  The format selector is populated at startup and probes the encoder before
  offering it, because some runtimes advertise a read-only loader as writable.
  JPEG XL uses the optional `libjxl` runtime library when GdkPixbuf cannot
  write JXL directly.

Annotation tools:

- Select/move existing annotations.
- Pen/freehand.
- Rectangle.
- Circle/ellipse.
- Line.
- Arrow.
- Text insertion opens a prompt and uses the configurable text size at the click
  position; selecting an existing text annotation and changing Text size
  resizes it.
- Blur/pixelate region.

Editor state is stored in `Rc<RefCell<EditorState>>`. Undo/redo currently tracks pushed annotations, not every state mutation.

## Important Implementation Notes

- `EditorState::background` is a `gdk_pixbuf::Pixbuf`; rendering happens over this base image.
- `editor::view_transform` computes fit-to-window or manual zoom scale plus centering offsets.
- Pointer coordinates are mapped from widget/view space to image space with `editor::map_to_image`.
- `editor::render_to_pixbuf` renders the final image for save/copy in image coordinates, without view zoom, fit offsets, or selection outlines.
- Blur is implemented as pixelation of the selected rectangle using nearest-neighbor downscale/upscale.
- Icons are SVG bytes with color replacement in `ui.rs`; theme changes recolor registered `gtk::Image` instances.
- Portal capture runs on a Tokio runtime. The GTK thread polls a standard `mpsc` receiver with `glib::timeout_add_local`.

## Build And Run

Host dependencies documented in `README.md` for Fedora:

```sh
sudo dnf install rust cargo gtk4-devel libadwaita-devel pkgconf-pkg-config gcc
```

Run locally:

```sh
cargo run
```

Install locally:

```sh
make install
```

After local install, a GNOME custom keyboard shortcut can run:

```sh
wayshot-gnome-capture
```

That command starts WayShot hidden, requests a screenshot through the desktop portal, then presents WayShot with the captured image loaded. After editing, `Ctrl+C` copies the rendered result through `wl-copy` and closes the window when that succeeds.

Flatpak:

```sh
flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json
flatpak-builder --force-clean --install-deps-from=flathub --user --install build-flatpak flatpak/io.github.gutopardini.wayshot.yml
```

When installed via Flatpak, the capture command is:
`flatpak run io.github.gutopardini.wayshot --capture`

`WAYSHOT_CAPTURE=1` remains a capture-first alias for existing shortcuts. The portal request starts before the editor is built. Esc in the screenshot UI exits without presenting the window.

## Flatpak Release Workflow

To update the Flatpak version (e.g., on Flathub or for local builds):

1. **Branding & Versioning:** Bump version in `Cargo.toml` and add a new entry to the `<releases>` list in `data/io.github.gutopardini.wayshot.metainfo.xml`.
2. **AppStream Screenshots:** References in `metainfo.xml` MUST use fixed tag URLs (e.g., `.../v0.1.6/...`) and never point to the `main` branch.
3. **Tagging:** Create and push a new git tag (e.g., `v0.1.6`) for the main repository. **Tag immutability is critical:** if a fix is needed after tagging, you MUST create a new version (e.g., `v0.1.7`).
4. **Manifest Update:**
    - Update the `wayshot` module in `flatpak/io.github.gutopardini.wayshot.yml` to point to the new `tag` and its exact `commit` hash.
    - **Dependency Stability:** NEVER change the `wl-clipboard` build system. It must remain `buildsystem: meson`.
5. **Sync to Flathub:** Copy the updated `.yml` and `cargo-sources.json` to the external `flathub-wayshot` repository and submit the PR.

   ```sh
   git tag v0.1.x
   git push origin v0.1.x
   ```
2. **Update Manifesto**: Edit `flatpak/io.github.gutopardini.wayshot.yml` to point to the new `tag` and its `commit` hash.
3. **Dependency Check**: If new external tools (like `wl-copy`) are needed, add them as modules in the manifesto.
4. **Vendoring**: If `Cargo.lock` changed, regenerate `flatpak/cargo-sources.json`:
   ```sh
   flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json
   ```

## Verification Commands

Use these for local verification before pushing or preparing a release:

```sh
# Formatting: ensure no trailing spaces and standard wrapping
cargo fmt --all -- --check

# Linting: check for collapsible ifs and other common issues
cargo clippy --all-targets -- -D warnings

# Build & Test
cargo test
cargo build --release --locked
```

Before pushing, run at least `cargo fmt --all -- --check`; CI enforces the same
formatting check, so import ordering and rustfmt-only changes should be caught
locally.

If version was bumped, ensure `Cargo.lock` is synced by running `cargo check`.

## Development Guidance

- Keep UI changes in `src/ui.rs` unless they alter editor semantics or rendering.
- Keep annotation data, geometry, hit testing, and Cairo rendering in `src/editor.rs`.
- Preserve the portal-based screenshot flow; Wayland/GNOME reliability is a core project goal.
- Prefer small, explicit changes over new abstractions. The current codebase is intentionally compact.
- Be careful with coordinate spaces. UI event handlers receive view coordinates; stored annotations use image coordinates.
- **Persistence:** Drawing settings (stroke width, color, and text size), behavior
  settings, and the selected output format are persisted via `glib::KeyFile` in
  `~/.config/wayshot/settings.ini`. Update both `load_settings` and
  `save_settings` when adding new persistent state.
- **Flatpak Releases:** 
  - When updating the version, sync `Cargo.toml`, `Cargo.lock`, and `metainfo.xml`.
  - Update the Flatpak manifest (`flatpak/io.github.gutopardini.wayshot.yml`) with the exact commit hash and tag.
  - The commit hash in the Flathub repository manifest MUST match the final commit hash of the tag in the main repository.
- When adding a new annotation type, update:
  - `Annotation`
  - `Tool` if it is user-selectable
  - drawing in `draw_annotation`
  - bounds in `annotation_bounds`
  - movement in `move_annotation`
  - UI tool button setup in `src/ui.rs`
- When adding assets, keep them under `assets/` or `data/` according to whether they are compile-time UI assets or packaging/install assets.
- Avoid destructive git commands. The repository may contain user edits during agent work.

## Known Gaps / Risks

- The current test suite is small and focuses on the optional JXL encoder;
  broader UI and portal integration tests are still absent.
- Text annotations can be added and edited through the text prompt; resizing is
  available through the settings popover when a text annotation is selected.
- Undo/redo is annotation-stack based and does not appear to cover selection movement as a distinct undo step, background replacement, or color/tool changes.
- Clipboard close-after-copy depends on `wl-copy` being available. If `wl-copy` fails, WayShot falls back to GTK clipboard and keeps the window open so clipboard ownership is not lost.
- Export formats are discovered from working runtime encoders;
  JPEG XL is offered only when the installed runtime provides a functioning
  JXL writer or `libjxl` encoder.
