# Glint Plugin

Reusable plugin contracts and a synchronous Boa 0.22 runtime. No GPUI, HTTP, SQLite or database driver dependency. `gpui-glint` re-exports this crate as `plugins` when its optional `plugins` feature is enabled; it also provides the native `components::PluginPage` renderer.

## Host boundary

1. Discover inert `glint-plugin.json` / `main.js` with `Plugin::load`. Entry paths must remain inside the plugin directory. API version 1 is required.
2. Inspect requested `capabilities` and `contributions`; reject unsupported kinds/permissions. Discovery is not approval or activation.
3. Register only the host APIs you intend to expose with `HostRegistry::register(name, required_capability, callback)`. API calls check capabilities in Rust, even when scripts call the low-level bridge directly.
4. Persist `Approval::for_plugin` after approval, then use `activate_approved` to reject changed code/permissions before evaluation. Alternatively activate with explicit fresh grants. Invoke a declared hook with JSON input, validate output against the application's contract, then commit changes. Drop the runtime to deactivate; create a fresh runtime per invocation for isolation.

`glint.on("database.page", callback)` declares a synchronous hook. `glint.host.call("database.describe", table)` invokes a host API. Promises are rejected by this first version. Relative/ES-module loading, Node APIs, networking, filesystem, process execution and dynamic native extensions are not provided. Generic diagnostic messages omit script-thrown values and host callback errors, which may contain secrets.

`HostRegistry::with_crypto()` offers `crypto.sha256Hex(text)` and `crypto.hmacSha256Hex(key, text)`, gated by `crypto`. Business signing algorithms remain in application plugins. Registry callbacks must capture ordinary host data, not engine/GC values; callbacks must not re-enter a runtime.

## Pages across applications

A `page` contribution names a hook; the host chooses where to mount it. Deserialize its output into `Page`, call `validate`, then render using `gpui_glint::components::PluginPage`. `PluginPageEvent` carries action/data back to the host. The renderer runs no JavaScript and grants no database access.

The v1 page schema supports `text`, `column`, `row`, `button`, and `table`. Native tables are paginated. Layout depth, node count, unique interactive IDs, table dimensions and data size are bounded. Unsupported schema is rejected. This is a page composition foundation, not arbitrary JavaScript access to GPUI entities or a full database IDE/editor.

`cargo run -p glint-plugin --example database_page` demonstrates a database metadata host with fixture rows; it opens no connection. A real database application implements `database.describe` using its connection service, assigns `database.metadata.read`, and separately grants query/write capabilities when those APIs exist. Commands can use the same contribution/hook mechanism without pages.

## Execution isolation

`Limits` bounds VM loops, recursion and input/output size, **not wall-clock time or total process memory**. Parsing, regex, native operations and allocations require host supervision for untrusted plugins. Boa contexts are thread-local; construct them inside the execution thread/process. Do not execute arbitrary plugins on the UI thread.

Glint API uses an auxiliary process with a three-second watchdog and bounded IPC. It exposes only selected environment values, crypto and header patches. This protects the UI and stops runaway execution, but is not an OS filesystem sandbox or a hard OS memory quota. No capability automatically exposes a system API: only registered Rust callbacks do.

`TYPESCRIPT_API` contains editor declarations for the generic boundary. Application-specific callback argument/output types belong to the application. The API is experimental; pin versions while integrating.

Declarative contributions may omit `hook` and provide an optional `data` JSON value. A manifest containing only such contributions may have empty `hooks`; the host must validate supported kinds and the data schema before use. Declaration data grants no runtime capabilities. Existing hook contributions serialize exactly as before when `data` is absent. Glint API uses this for inert `oauth.preset` configuration and never activates a JavaScript runtime to select a preset.
