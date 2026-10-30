# Windows packaging

`build-windows.ps1` is the shared Windows x64 release and Inno Setup pipeline. `products.psd1` owns product identity and paths; `setup.iss` owns common installer behavior. Makefile and CI invoke this pipeline directly; there are no crate-local forwarding scripts or templates.

```powershell
make win-installer PRODUCT=api
make win-installer PRODUCT=workbench
# Inspect without compiling, downloading or creating output directories:
./packaging/windows/build-windows.ps1 -Product workbench -Plan
# Check both products; optionally compile templates using harmless fixture payloads:
./packaging/windows/test-packaging.ps1 -IsccPath 'C:/path/to/ISCC.exe'
```

`PRODUCT` defaults to `workbench`.

`-Version` must match the selected crate's version. `-IsccPath` and the Makefile `VERSION`, `ISCC`, `POWERSHELL` overrides remain supported. Release builds retain static MSVC runtime linking, Microsoft WebView2 signature verification and SHA-256 checksums.

The API and Workbench keep their original AppId, AppUserModelID, installation directory, shortcuts and artifact names. Their staging/output directories are independent. Add another product by defining its metadata and an allowed `-Product` value, rather than copying the pipeline or template.

The template smoke check only compiles installers in `target/windows-packaging-tests`; it does not install, uninstall, run the fixture payloads or replace release outputs. Full install/upgrade/uninstall checks still use the API's existing `test-installer.ps1` on a Windows test machine.
