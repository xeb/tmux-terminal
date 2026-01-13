PREFIX = $(HOME)/bin/tmux-terminal
SERVICE_DIR = $(HOME)/.config/systemd/user
SERVICE_NAME = tmux-terminal.service

.PHONY: build install uninstall update start stop restart status

build:
	cargo build --release

install: build
	@echo "Installing tmux-terminal..."
	mkdir -p $(PREFIX)
	mkdir -p $(SERVICE_DIR)
	cp target/release/tmux-terminal $(PREFIX)/
	cp -r static $(PREFIX)/
	cp tmux-terminal.service $(SERVICE_DIR)/
	systemctl --user daemon-reload
	systemctl --user enable $(SERVICE_NAME)
	systemctl --user start $(SERVICE_NAME)
	@echo "Installed and started tmux-terminal service"
	@echo "Access at http://localhost:5533"

uninstall:
	@echo "Uninstalling tmux-terminal..."
	-systemctl --user stop $(SERVICE_NAME)
	-systemctl --user disable $(SERVICE_NAME)
	rm -f $(SERVICE_DIR)/$(SERVICE_NAME)
	rm -rf $(PREFIX)
	systemctl --user daemon-reload
	@echo "Uninstalled tmux-terminal"

update:
	@echo "Updating tmux-terminal..."
	git pull
	cargo build --release
	cp target/release/tmux-terminal $(PREFIX)/
	cp -r static $(PREFIX)/
	systemctl --user restart $(SERVICE_NAME)
	@echo "Updated and restarted tmux-terminal"

start:
	systemctl --user start $(SERVICE_NAME)

stop:
	systemctl --user stop $(SERVICE_NAME)

restart:
	systemctl --user restart $(SERVICE_NAME)

status:
	systemctl --user status $(SERVICE_NAME)
