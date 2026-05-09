.PHONY: install install-shortcut

PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
APPDIR := $(PREFIX)/share/applications
ICONDIR := $(PREFIX)/share/icons/hicolor/scalable/apps
PNGICONDIR_256 := $(PREFIX)/share/icons/hicolor/256x256/apps
PNGICONDIR_512 := $(PREFIX)/share/icons/hicolor/512x512/apps
PNGICONDIR_1024 := $(PREFIX)/share/icons/hicolor/1024x1024/apps

install:
	cargo build --release
	install -Dm755 target/release/wayshot $(BINDIR)/wayshot
	install -Dm755 scripts/wayshot-gnome-capture $(BINDIR)/wayshot-gnome-capture
	install -Dm644 data/io.github.gutopardini.wayshot.desktop $(APPDIR)/io.github.gutopardini.wayshot.desktop
	sed -i 's|^Exec=.*|Exec=$(BINDIR)/wayshot %f|' $(APPDIR)/io.github.gutopardini.wayshot.desktop
	rm -f $(ICONDIR)/io.github.gutopardini.wayshot.svg
	install -Dm644 assets/app-icons/256x256/apps/io.github.gutopardini.wayshot.png $(PNGICONDIR_256)/io.github.gutopardini.wayshot.png
	install -Dm644 assets/app-icons/512x512/apps/io.github.gutopardini.wayshot.png $(PNGICONDIR_512)/io.github.gutopardini.wayshot.png
	install -Dm644 assets/app-icons/1024x1024/apps/io.github.gutopardini.wayshot.png $(PNGICONDIR_1024)/io.github.gutopardini.wayshot.png
	-update-desktop-database $(APPDIR)
	@printf '%s\n' \
		'[Icon Theme]' \
		'Name=Hicolor' \
		'Comment=Fallback icon theme' \
		'Directories=256x256/apps,512x512/apps,1024x1024/apps' \
		'' \
		'[256x256/apps]' \
		'Size=256' \
		'Type=Fixed' \
		'Context=Applications' \
		'' \
		'[512x512/apps]' \
		'Size=512' \
		'Type=Fixed' \
		'Context=Applications' \
		'' \
		'[1024x1024/apps]' \
		'Size=1024' \
		'Type=Fixed' \
		'Context=Applications' \
		> $(PREFIX)/share/icons/hicolor/index.theme
	-gtk-update-icon-cache $(PREFIX)/share/icons/hicolor

install-shortcut:
	scripts/install-gnome-shortcut
