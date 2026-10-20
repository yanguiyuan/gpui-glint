.DEFAULT_GOAL := help

ifeq ($(OS),Windows_NT)
POWERSHELL ?= powershell.exe
else
POWERSHELL ?= pwsh
endif

.PHONY: help win-setup

help:
	@echo make win-setup - Build the Glint API Windows x64 setup and SHA-256 file
	@echo Optional overrides: VERSION=0.1.5 ISCC="path/to/ISCC.exe" POWERSHELL=pwsh

win-setup:
	"$(POWERSHELL)" -NoProfile -File crates/glint-api/packaging/windows/build-windows.ps1 $(if $(VERSION),-Version "$(VERSION)") $(if $(ISCC),-IsccPath "$(ISCC)")
