# SDK registries

A registry is a JSON file, `index.json`, that lists SDKs and where to download them.
The SDK Manager (File > SDK Manager…) and `mapcraft-cli sdk` read it to install and update SDKs.
The official registry is the MapCraftSDKs repository; anyone can host another one in the same format, on a web server or in a folder.

## Where MapCraft looks

Preferences > SDK registries lists the registries, the official one first.
Each entry is an `https://` or `http://` URL, or a path to an `index.json` on disk.
Use a local path for a private registry or for testing.

## Format

```json
{
  "format": "mapcraft.sdk-registry",
  "version": 1,
  "name": "MapCraft SDKs",
  "sdks": [
    {
      "id": "surfsup",
      "name": "SurfsUp",
      "version": "3",
      "description": "Surf maps for SurfsUp v3.",
      "author": "MapCraft",
      "license": "MIT",
      "homepage": "https://example.com/surfsup",
      "download": "https://example.com/releases/surfsup-3.zip",
      "sha256": "9f2c...64 hex digits...",
      "size": 193512,
      "min_mapcraft": "0.2.0",
      "plugins": ["godot", "source-bsp", "surfsup"]
    }
  ]
}
```

| Key | Meaning |
|---|---|
| `format` | Always `mapcraft.sdk-registry` |
| `version` | Registry format version; MapCraft refuses a version newer than it reads (currently 1) |
| `name` | Shown above the registry's SDKs |
| `sdks` | The entries |

Each entry:

| Key | Required | Meaning |
|---|---|---|
| `id` | yes | The SDK's id from its `sdk.toml`; lowercase letters, digits, `_` and `-`; unique in the registry |
| `name` | yes | Display name |
| `version` | yes | The SDK's version; numbers separated by dots compare numerically, anything else counts as different |
| `description` | no | One or two sentences |
| `author` | no | Who maintains it |
| `license` | no | SPDX expression, such as `MIT` or `CC-BY-4.0` |
| `homepage` | no | Where to read more or report problems |
| `download` | yes | The zip's URL, or a path relative to the registry file |
| `sha256` | yes | SHA-256 of the zip, 64 hex digits |
| `size` | yes | Size of the zip in bytes, at most 512 MiB |
| `min_mapcraft` | no | The oldest MapCraft version the SDK works with |
| `plugins` | no | Ids of the plugins the zip ships |

Unknown keys are ignored, so later versions can add keys without breaking older readers.
A registry with a missing required key, a bad id, a duplicate id or a malformed checksum is refused as a whole, so a mistake is caught before anyone installs from it.

A relative `download` resolves against the registry's own location: next to a local `index.json`, or beside the registry URL.
Entries in a remote registry always resolve to URLs, so a remote registry can never make MapCraft read a local file.

## The zip

The zip holds one SDK folder, either at its root or inside a single folder.
`mapcraft-cli sdk package <dir>` makes one: every file except hidden ones, under `<id>/`, with fixed timestamps so the same files always give the same checksum.
It prints the entry to add to `index.json`, with `sha256`, `size` and `min_mapcraft` filled in from the archive and `sdk.toml`.

## Installing

Installing downloads the zip (on a background thread in the SDK Manager), checks its size and SHA-256, and unpacks it into `sdks/<id>` in the user's data folder.
That is `~/.local/share/mapcraft/sdks` on Linux, `%LOCALAPPDATA%\mapcraft\sdks` on Windows and `~/Library/Application Support/mapcraft/sdks` on macOS.

- Entries with absolute paths, `..`, drive letters or links are refused, so an archive cannot write outside its folder.
- At most 20,000 files and 2 GiB unpacked, counted from the bytes written rather than the sizes the archive claims.
- The zip must hold an `sdk.toml` whose `id` matches the entry.
- The SDK unpacks into a staging folder first; an older copy is swapped out only once the new one has loaded, and comes back if the swap fails.
- An SDK installed or updated this way asks again before its compile profiles first run a program.

User SDKs load before bundled ones, so an update from a registry replaces a copy shipped with MapCraft without touching it.

## Removing

Only SDKs in the user's data folder can be removed.
SDKs next to the MapCraft executable, in the source tree or in folders from the preferences are never deleted.

## Checks

The SDK Manager's Check button and `mapcraft-cli sdk check` run the same checks, each reported as pass, warn or fail:

- `sdk.toml` parses.
- Each FGD exists and parses without diagnostics.
- Each listed plugin's `plugins/<id>.wasm` exists, loads in the plugin host and reports the same id.
- Each path setting is set (warn when unset) and exists (fail when set but missing).
- Each texture folder exists, with its material count.
- Each program a compile profile runs exists, as a path or on `PATH`.
- Each map in `samples/` loads and passes the map validators with the SDK's FGDs.

Unset paths only warn, because they belong to each user's machine; anything that is set but broken fails.
`mapcraft-cli sdk check` exits with an error when any item fails, which is what the MapCraftSDKs review workflow relies on.

## Command line

```sh
mapcraft-cli sdk list [--registry <url|path>] [--data <dir>]
mapcraft-cli sdk check <dir|id> [--var name=value]... [--data <dir>]
mapcraft-cli sdk install <id> [--registry <url|path>] [--data <dir>]
mapcraft-cli sdk remove <id> [--data <dir>]
mapcraft-cli sdk package <dir> [--out <dir>] [--url <base>]
mapcraft-cli sdk new <dir> --id <id> --name <name>
```

`--data` replaces the user's data folder, which is how CI installs into a temporary one.
`--url` prefixes the zip's file name in the printed `download`, for registries that host zips elsewhere.
`sdk new` writes the same template the SDK Manager's Create tab uses.
