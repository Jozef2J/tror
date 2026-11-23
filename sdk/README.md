# TrorPad plugin SDK

`trorpad-plugin` is the Rust SDK for TrorPad plugins. A plugin is compiled to WebAssembly (`wasm32-unknown-unknown`) and runs in a sandbox inside TrorPad (the [wasmi](https://github.com/wasmi-labs/wasmi) interpreter, no JIT). A plugin can only reach what its permissions allow, and it never sees passwords.

- SDK crate: [`trorpad-plugin/`](trorpad-plugin/)
- Full example: [`examples/text-tools/`](examples/text-tools/) — sorting, JSON formatting, wrapping in a tag, case changes, a word-frequency panel, a TODO table and editor decorations. It is built in CI and attached to every release as `trorpad-plugin-text-tools.zip`.

## Quick start

```toml
# Cargo.toml
[package]
name = "hello"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
trorpad-plugin = { git = "https://github.com/Jozef2J/tror" }
```

```rust
// src/lib.rs
use trorpad_plugin::{Answer, Plugin, Value, host, ui};

#[derive(Default)]
struct Hello;

impl Plugin for Hello {
    fn command(&mut self, _id: &str) -> Result<(), String> {
        host::prompt("tag", "Tag", "div")
    }

    fn answer(&mut self, _id: &str, answer: Answer) -> Result<(), String> {
        let text = host::selected_text()?;
        host::replace_selection(&format!("<{0}>{text}</{0}>", answer.value))
    }

    fn panel_open(&mut self, panel: &str) -> Result<(), String> {
        host::panel(panel, ui::column(vec![ui::heading("Hello"), ui::primary("go", "Press me")]))
    }

    fn panel_event(&mut self, _panel: &str, id: &str, _value: &Value) -> Result<(), String> {
        host::notify(id)
    }
}

trorpad_plugin::export!(Hello);
```

```
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

Put `plugin.json` and the built file (renamed to `plugin.wasm`) into a zip and install it with **Plugins → Manage plugins… → Install from zip…**, or copy the folder into `<data folder>/plugins/<name>/`.

## Manifest: `plugin.json`

```json
{
  "id": "hello",
  "name": "Hello",
  "version": "0.1.0",
  "wasm": "plugin.wasm",
  "permissions": ["fs"],
  "events": ["saved", "opened", "changed", "activeChanged"],
  "commands": [{ "id": "wrap", "title": "Wrap in tag…", "key": "Ctrl+Alt+W" }],
  "panels": [{ "id": "words", "title": "Frequent words", "dock": "left" }],
  "settings": [{ "key": "indent", "title": "JSON indent", "type": "choice", "options": ["2", "4", "tab"], "default": "4" }]
}
```

| Field | Meaning |
| --- | --- |
| `id`, `name`, `version` | Identity shown in the plugin manager. `id` must be unique. |
| `wasm` | The module file inside the plugin folder. |
| `permissions` | `fs`, `network`, `hosts`, `db` — see below. Shown to the user before installation. |
| `events` | Events the plugin subscribes to: `saved`, `opened`, `changed` (0.4 s after the last edit, once per version), `activeChanged`. |
| `commands` | Menu items under **Plugins**. `key` is an optional shortcut. |
| `panels` | Dockable panels. `dock`: `left` (default), `bottom` or `center`. |
| `settings` | Typed settings (`text`, `bool`, `number`, `choice`) edited in **Settings…** of the plugin; read with `host::setting`. |

## Permissions

| Permission | SDK functions | What it gives |
| --- | --- | --- |
| *(none)* | text, selection, panels, dialogs, notifications, own storage | Everything that only touches the active document and the plugin's own data. |
| `fs` | `host::read_file`, `host::write_file` | Read and write files inside the open project folder only. Paths are resolved, symbolic links included; escaping the folder is refused. |
| `network` | `host::http`, `host::get` | HTTP and HTTPS only. 20 s timeout, responses up to 10 MB. 4xx/5xx are returned, not treated as errors. |
| `hosts` | `host::hosts`, `remote_list`, `remote_read`, `remote_write` | Server profiles (name, protocol, host, folder). TrorPad connects with the password from the OS credential store; the plugin never sees it. Unknown SFTP host keys are refused. |
| `db` | `host::databases`, `host::db_query` | Run SQL on saved database profiles: `{columns, rows, affected, truncated}`. |

## API overview

- **`Plugin` trait:** `command` (required), `event`, `answer`, `panel_open`, `panel_event` (optional, empty by default).
- **`host`:**
  - document: `text`, `selection`, `selected_text`, `info`, `replace`, `replace_selection`, `select`;
  - messages: `notify`, `log`, `log_error`;
  - files and data: `open_tab`, `read_file`, `write_file`, `storage_get`, `storage_set`, `setting`;
  - dialogs: `prompt`, `pick` (the result arrives in `Plugin::answer`);
  - panels: `panel`, `show_panel`;
  - decorations: `decorate(key, &[Decoration])`, `clear_decorations(key)` — `Decoration::highlight`, `underline`, `gutter`, with `.color("#rrggbb")` and `.message(...)` (hover text); up to 5000 per key;
  - API 2: `http`, `get`, `hosts`, `remote_list`, `remote_read`, `remote_write`, `databases`, `db_query`.
- **`ui`** (panel elements): `column`, `row`, `heading`, `text`, `strong`, `small`, `code`, `button`, `primary`, `input`, `multiline`, `checkbox`, `select`, `list`, `progress`, `separator`, `space`, `table`, `tree`, `node`. Inputs send an event on Enter or when focus leaves with a changed value; checkboxes, selects, lists, tables and trees send events at once.
- **`text`:** helpers for character positions (`byte_of`, `slice`, `lines_span`, `chars`). All positions in the API are counted in characters, not bytes.

## Limits

- One call may use up to 4·10⁹ fuel units (about 8 s on a modest machine). A plugin that exceeds it is stopped until **Reload**.
- Memory: up to 512 MB per plugin.
- Each plugin runs in its own thread. Edits are applied only if the document did not change while the plugin was working, and as one undo step.
- A panic is written to the log and does not crash TrorPad.

## Stability promise

The plugin API has a version number, sent to the plugin in `initialize {api, app}`. The current version is **API 2** (TrorPad 0.26 and later).

- Within API 2, nothing is removed or changed in a breaking way: method names, parameter and result shapes, event names and the binary interface stay as documented. New methods, events, fields and UI elements may be added; plugins should ignore fields they do not know.
- A plugin built for API 1 or API 2 keeps working in every TrorPad release that supports API 2.
- If a breaking change is ever needed, it will come as **API 3**. TrorPad will then support API 2 and API 3 side by side for at least two minor releases, and the change will be announced in the release notes.
- The SDK crate follows Semantic Versioning: `1.x` releases are compatible with each other.

## Binary interface (for other languages)

The SDK hides this, but any language that compiles to WebAssembly can implement it:

- The module exports `memory`, `trorpad_alloc(len) -> ptr`, `trorpad_free(ptr, len)` and `trorpad_handle(ptr, len) -> u64` (0 means success, otherwise `ptr << 32 | len` of a `{"error": ...}` answer).
- It imports `call(ptr, len) -> answer_len` and `take(ptr)` from the module `trorpad`.
- Messages are JSON `{"method", "params"}`; answers are `{"ok": value}` or `{"error": text}`.
- Host → plugin: `initialize {api, app}`, `command {id}`, `event {name, ...}` with `answer {id, value, index?}`, `panel_open {panel}`, `panel {panel, id, value}`, `settings`, `saved`/`opened {path}`, `changed`, `activeChanged`.
