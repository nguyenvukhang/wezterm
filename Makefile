current:
	@make debug

analyze:
	python3 analyze.py

debug:
	cargo build
	mkdir -p $(RUN_DIR)
	rm -rf $(RUN_DIR)/*
	cp ./target/debug/wezterm $(RUN_DIR)
	cp ./target/debug/wezterm-gui $(RUN_DIR)
	@make size
	# open Alatty.app

install:
	cargo build --release
	mkdir -p $(RUN_DIR)
	rm -rf $(RUN_DIR)/*
	cp ./target/release/wezterm $(RUN_DIR)
	cp ./target/release/wezterm-gui $(RUN_DIR)
	rm -rf /Applications/Alatty.app
	cp -a $(APP_DIR) /Applications/Alatty.app
	@make size

size:
	du -sh $(APP_DIR)
	du -s $(APP_DIR)

d:
	@make debug

i:
	@make install

o:
	open Alatty.app

a:
	@make analyze

u:
	cargo +nightly udeps

MAKEFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
MAKEFILE_DIR  := $(dir $(MAKEFILE_PATH))
APP_DIR := $(MAKEFILE_DIR)Alatty.app
RUN_DIR := $(APP_DIR)/Contents/MacOS
