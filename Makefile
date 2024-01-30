current:
	make analyze

analyze:
	python3 analyze.py

debug:
	cargo build
	mkdir -p $(RUN_DIR)
	rm -rf $(RUN_DIR)/*
	cp ./target/debug/wezterm $(RUN_DIR)
	cp ./target/debug/wezterm-gui $(RUN_DIR)
	open Alatty.app

install:
	cargo build --release
	mkdir -p $(RUN_DIR)
	rm -rf $(RUN_DIR)/*
	cp ./target/release/wezterm $(RUN_DIR)
	cp ./target/release/wezterm-gui $(RUN_DIR)
	rm -rf /Applications/Alatty.app
	cp -a $(APP_DIR) /Applications/Alatty.app

d:
	@make debug

i:
	@make install

MAKEFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
MAKEFILE_DIR  := $(dir $(MAKEFILE_PATH))
APP_DIR := $(MAKEFILE_DIR)Alatty.app
RUN_DIR := $(APP_DIR)/Contents/MacOS
