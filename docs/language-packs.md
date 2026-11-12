# External syntax language packs

Glint loads data-only TextMate JSON grammars from a local directory. File previews,
Markdown code fences and diffs use the same tokenization entry point. Existing built-in
lexers remain available for languages that have no installed pack.

## Install

Copy a pack directory into `<agent data directory>/syntax/`, then restart the application.
On Windows the default is `%APPDATA%\Glint Agent\syntax\`. `GLINT_AGENT_DATA_DIR` moves
the agent data directory; `GLINT_SYNTAX_DIR` overrides the syntax directory directly.

For the included JSON example, run from the repository in PowerShell:

```powershell
$syntaxDir = if ($env:GLINT_SYNTAX_DIR) { $env:GLINT_SYNTAX_DIR } elseif ($env:GLINT_AGENT_DATA_DIR) { Join-Path $env:GLINT_AGENT_DATA_DIR 'syntax' } else { Join-Path $env:APPDATA 'Glint Agent/syntax' }
New-Item -ItemType Directory -Force -Path $syntaxDir | Out-Null
Copy-Item -LiteralPath ./examples/language-packs/json -Destination $syntaxDir -Recurse
```

Remove the pack directory and restart to uninstall. No package is installed automatically.
The example declares a single `json` language and demonstrates property names, strings
with escapes, numbers and the `true`/`false`/`null` literals. It reuses the built-in
`json` language id, so installing it replaces Glint's built-in JSON highlighting for
`.json` files and `json` code fences; delete the directory and restart to get the
built-in highlighting back. The grammar is deliberately small rather than a complete JSON
definition: braces, brackets, commas and colons stay plain, and invalid documents are
tokenized exactly like valid ones.

## Pack format

Each immediate child directory has a `glint-plugin.json` manifest:

```json
{
  "id": "example.syntax.mydsl",
  "name": "My DSL",
  "version": "0.1.0",
  "api_version": 1,
  "entry": "main.js",
  "hooks": [],
  "capabilities": [],
  "contributions": [{
    "id": "mydsl",
    "kind": "syntax.language",
    "title": "My DSL",
    "data": {
      "language": "mydsl",
      "extensions": ["dsl"],
      "aliases": ["my-dsl"],
      "filenames": ["MyDslfile"],
      "grammar": "syntaxes/mydsl.tmLanguage.json"
    }
  }]
}
```

`entry` is retained for compatibility with the general plugin manifest; syntax loading
never reads or evaluates it. A pack may contain several language contributions. Hooks
and other contribution kinds are rejected by this loader. Paths must be relative and
resolve inside the pack directory, including through symlinks.

Use `extensions` without the leading dot. Language IDs, aliases and extensions use
ASCII letters/digits and `._+-`; IDs must be lowercase. Aliases and extensions are
matched case-insensitively. Exact filenames are case-sensitive and override extension
associations. File extensions also act as language aliases for diff/code-fence callers.
Filename-only associations apply when a file path is available, such as file previews.

Grammars are JSON TextMate definitions with a unique `scopeName`. `include` can reference
another installed grammar by scope (e.g. `source.json` from another pack). No default grammar library is
bundled by the TextMate engine; embedded grammar dependencies must be installed too.
An existing VS Code extension's grammar files can be reused subject to their license;
the extension's JavaScript, LSP and package activation are not supported.

Standard scope categories map to Glint's current light/dark code palette: `keyword` and
`storage`, `string`, `comment`, `constant.numeric`, other `constant`, `entity` and
`support`. Unmapped scopes use the surrounding recognized scope or plain text.

## Loading and limits

The application takes one immutable registry snapshot at startup. Packages load in
directory-name order. Duplicate language IDs, aliases and grammar scopes are rejected;
the first valid package wins. A failed package is discarded atomically. Other packages
continue loading. Diagnostics are exposed by `LanguageRegistry::diagnostics` and logged
to stderr by the default host integration.

Manifest size is limited to 64 KiB, each grammar to 512 KiB, each package to 32
contributions and the registry to 128 languages. At most 128 child directories are
loaded. Buffers above 2 MiB and lines above 16 KiB fall back to plain text. UTF-8 byte
ranges and original line endings are preserved.

Install grammars from trusted sources: grammar regexes run in-process; these size
limits are not a wall-clock timeout or an OS sandbox.

Library hosts can use `gpui_glint::syntax::LanguageRegistry::load_dir` to inspect a pack
directory and `language_for_path` to resolve file associations. The default renderers
use `gpui_glint::syntax::global()`; configure its environment before first use.
