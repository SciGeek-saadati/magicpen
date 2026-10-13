PREFIX ?= $(HOME)/.local
BIN = target/release/magicpen

.PHONY: build install uninstall

build:
	cargo build --release --locked

install: build
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/magicpen
	install -Dm644 assets/magicpen.desktop $(DESTDIR)$(PREFIX)/share/applications/magicpen.desktop
	install -Dm644 assets/magicpen.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/magicpen.svg

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/magicpen
	rm -f $(DESTDIR)$(PREFIX)/share/applications/magicpen.desktop
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/magicpen.svg
