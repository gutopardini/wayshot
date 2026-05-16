# Flathub submission

This document is for maintainers preparing a Flathub submission.

## Validate metadata

```sh
appstreamcli validate data/io.github.gutopardini.wayshot.metainfo.xml
```

If `org.flatpak.Builder` is installed, also run the Flathub-specific lint:

```sh
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream data/io.github.gutopardini.wayshot.metainfo.xml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest flatpak/io.github.gutopardini.wayshot.yml
```

## Submission source

The manifest references the GitHub repo:

```text
https://github.com/gutopardini/wayshot
```

For Flathub stable, publish a normal GitHub release first and submit a manifest that builds from that release tag or a fixed commit instead of the moving `main` branch. Also update screenshot URLs in the MetaInfo to use the tag or commit rather than `main`. Keep `cargo-sources.json` in the submission because Flathub builds do not have network access while compiling Rust crates.

## Compatibility metadata

Desktop eligibility is declared in `data/io.github.gutopardini.wayshot.metainfo.xml` using AppStream device support tags:

- `keyboard`
- `pointing`
- `display_length >= 768`

The Flathub page text also states that WayShot is designed and supported only for GNOME running on Wayland. AppStream can communicate this clearly in the store, but Flatpak does not provide a hard GNOME-vs-KDE install filter.

## Submit to Flathub

1. Fork `flathub/flathub` on GitHub.
2. Clone the `new-pr` branch:

   ```sh
   git clone --branch=new-pr git@github.com:YOUR_GITHUB_USERNAME/flathub.git
   cd flathub
   git checkout -b add-wayshot
   ```

3. Copy the required Flatpak files to the repository root:

   ```sh
   cp /path/to/wayshot/flatpak/io.github.gutopardini.wayshot.yml .
   cp /path/to/wayshot/flatpak/cargo-sources.json .
   ```

4. In `io.github.gutopardini.wayshot.yml`, replace `branch: main` with the release tag or fixed commit you just pushed.
5. Commit and push, then open a pull request against the `new-pr` base branch with the title `Add io.github.gutopardini.wayshot`.
6. After reviewer comments are addressed, trigger a test build in the pull request with `bot, build`.
