# dsh-pet desktop adapter

This package uses the Workbench managed Electron adapter contract on Windows x64. It contains the upstream release metadata and the bridge that exposes the pet's authenticated local endpoint. It requires neither DSH nor a separate Node.js installation.

Workbench supplies the download and progress UI, SHA-256 verification, ZIP installation, Windows process cleanup, and the current Agent session's busy or idle state. The adapter declares the executable, the loopback request path and token header, and the JSON body and expected response for the busy request. Idle sends DELETE to the same path.

The bundled adapter is selected by default. To use a separately distributed copy, set `GLINT_DESKTOP_ADAPTER_DIR` to an absolute directory containing `adapter.json` and `bootstrap.cjs`, then start Workbench. The setting selects one trusted local package. Click **启用** in **插件管理** to run it with the declared `managed-process` permission.

The bootstrap receives `GLINT_ADAPTER_PROFILE` and `GLINT_ADAPTER_RUNTIME`. It publishes `endpoint.json` atomically in the runtime directory with its PID, loopback port, and token. Workbench checks the PID and consumes the file once. The executable and archive root must be single path components. The ZIP must contain the Electron executable and `resources/app.asar` under its declared root.

Change the release version when changing the upstream asset. A bridge-only update can keep the same version. Workbench refreshes the bridge on the next start, including for an existing dsh-pet installation.
