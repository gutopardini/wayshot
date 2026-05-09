.PHONY: install install-shortcut

PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
APPDIR := $(PREFIX)/share/applications
ICONDIR := $(PREFIX)/share/icons/hicolor/scalable/apps

install:
	cargo build --release
	install -Dm755 target/release/wayshot $(BINDIR)/wayshot
	install -Dm755 scripts/wayshot-gnome-capture $(BINDIR)/wayshot-gnome-capture
	install -Dm644 data/io.github.gutopardini.wayshot.desktop $(APPDIR)/io.github.gutopardini.wayshot.desktop
	sed -i 's|^Exec=.*|Exec=$(BINDIR)/wayshot %f|' $(APPDIR)/io.github.gutopardini.wayshot.desktop
	install -Dm644 data/icons/hicolor/scalable/apps/io.github.gutopardini.wayshot.svg $(ICONDIR)/io.github.gutopardini.wayshot.svg
	-update-desktop-database $(APPDIR)
	@test -f $(PREFIX)/share/icons/hicolor/index.theme || printf '%s\n' \
		'[Icon Theme]' \
		'Name=Hicolor' \
		'Comment=Fallback icon theme' \
		'Directories=scalable/apps' \
		'' \
		'[scalable/apps]' \
		'Size=128' \
		'Type=Scalable' \
		'MinSize=1' \
		'MaxSize=512' \
		'Context=Applications' \
		> $(PREFIX)/share/icons/hicolor/index.theme
	-gtk-update-icon-cache $(PREFIX)/share/icons/hicolor

install-shortcut:
	scripts/install-gnome-shortcut
