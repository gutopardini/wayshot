.PHONY: install install-shortcut

PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
APPDIR := $(PREFIX)/share/applications
ICONDIR := $(PREFIX)/share/icons/hicolor/scalable/apps
PNGICONDIR := $(PREFIX)/share/icons/hicolor/1024x1024/apps

install:
	cargo build --release
	install -Dm755 target/release/wayshot $(BINDIR)/wayshot
	install -Dm755 scripts/wayshot-gnome-capture $(BINDIR)/wayshot-gnome-capture
	install -Dm644 data/io.github.gutopardini.wayshot.desktop $(APPDIR)/io.github.gutopardini.wayshot.desktop
	sed -i 's|^Exec=.*|Exec=$(BINDIR)/wayshot %f|' $(APPDIR)/io.github.gutopardini.wayshot.desktop
	rm -f $(ICONDIR)/io.github.gutopardini.wayshot.svg
	install -Dm644 assets/icons/wayshot.png $(PNGICONDIR)/io.github.gutopardini.wayshot.png
	-update-desktop-database $(APPDIR)
	@test -f $(PREFIX)/share/icons/hicolor/index.theme || printf '%s\n' \
		'[Icon Theme]' \
		'Name=Hicolor' \
		'Comment=Fallback icon theme' \
		'Directories=1024x1024/apps' \
		'' \
		'[1024x1024/apps]' \
		'Size=1024' \
		'Type=Fixed' \
		'Context=Applications' \
		> $(PREFIX)/share/icons/hicolor/index.theme
	-gtk-update-icon-cache $(PREFIX)/share/icons/hicolor

install-shortcut:
	scripts/install-gnome-shortcut
