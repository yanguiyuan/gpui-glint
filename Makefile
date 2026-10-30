.DEFAULT_GOAL := help

ifeq ($(OS),Windows_NT)
POWERSHELL ?= powershell.exe
else
POWERSHELL ?= pwsh
endif

PRODUCT ?= workbench

.PHONY: help win-installer

help:
	@echo make win-installer PRODUCT=workbench - Build Windows x64 setup and SHA-256 file
	@echo Products: workbench default, api
	@echo Optional overrides: VERSION=0.1.0 ISCC="path/to/ISCC.exe" POWERSHELL=pwsh

win-installer:
	"$(POWERSHELL)" -NoProfile -File packaging/windows/build-windows.ps1 -Product "$(PRODUCT)" $(if $(VERSION),-Version "$(VERSION)") $(if $(ISCC),-IsccPath "$(ISCC)")
