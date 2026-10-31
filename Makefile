.DEFAULT_GOAL := help

ifeq ($(OS),Windows_NT)
POWERSHELL ?= powershell.exe
else
POWERSHELL ?= pwsh
endif

PRODUCT ?= workbench

.PHONY: help win-installer macos-installer mac-installer

help:
	@echo make macos-installer PRODUCT=workbench - Build macOS release app, DMG and SHA-256 file
	@echo make win-installer PRODUCT=workbench - Build Windows x64 setup and SHA-256 file
	@echo Products: workbench default, api
	@echo macOS signing: GLINT_CODESIGN_IDENTITY="Developer ID Application: ..."
	@echo Windows overrides: VERSION=0.1.0 ISCC="path/to/ISCC.exe" POWERSHELL=pwsh

win-installer:
	"$(POWERSHELL)" -NoProfile -File packaging/windows/build-windows.ps1 -Product "$(PRODUCT)" $(if $(VERSION),-Version "$(VERSION)") $(if $(ISCC),-IsccPath "$(ISCC)")

# Native architecture; defaults to local ad-hoc signing.
macos-installer:
	bash packaging/macos/build-macos.sh "$(PRODUCT)"

mac-installer: macos-installer
