# MapCraft

An open-source level editor written in pure Rust, combining the best of Valve Hammer and TrenchBroom with a Godot-style scene tree, inspector and prefabs.

MapCraft knows no game by itself.
Each game is an **SDK**: entity definitions (FGD), material locations, compile profiles and **plugins**, sandboxed WebAssembly components that teach MapCraft how to read the game's assets and formats or add tools such as generators.
Plugins can be written in any language that compiles to a WebAssembly component: Rust, C, C++, C#, Go, Python, JavaScript or Zig.

First-party SDKs:

- **SurfsUp**: author maps that load straight into SurfsUp through the MapCraft Godot extension, with surf ramp and bhop generators.
- **Counter-Strike: Source**: open and save Hammer and Hammer++ `.vmf` files, browse VPK materials, run the Valve compilers.
- **Quake**: Standard and Valve220 `.map` files.

## Where it stands

Early development.
See [ROADMAP.md](ROADMAP.md) for what works today and what comes next.

## Building

Requires Rust 1.95 or newer and the `wasm32-wasip2` target for the first-party plugins.

```sh
rustup target add wasm32-wasip2
cargo xtask plugins
cargo build --release
cargo xtask ci
```

`cargo xtask plugins` builds the plugins in `plugins/` and copies each into the SDKs that use it.
See [docs/plugin-api.md](docs/plugin-api.md) to write your own.

## License

MIT, see [LICENSE](LICENSE).
