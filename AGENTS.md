# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Overview

A cross-platform image viewer built with **Bevy 0.18** and Rust. Single-file application (`src/main.rs`, ~1700 lines) using Bevy's Entity-Component-System architecture. Supports comparing images side-by-side, in grids, stacked, or in horizontal/vertical layouts.

## Build & Development Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build (binary: image-viewer)
cargo run -- img1.png img2.png # Run with image arguments
cargo fmt                      # Format code (see rustfmt.toml)
cargo clippy                   # Lint (no clippy.toml; use defaults)
cargo bundle --release         # Create macOS app bundle (requires cargo-bundle)
```

**There are no tests in this project.** No test files, no `#[cfg(test)]` modules, no integration tests directory.

## Architecture

### File Structure

All application code lives in `src/main.rs`.

### Code Organization (top to bottom in main.rs)

Imports -> Type alias -> CLI args (clap) -> Constants -> App states -> Config structs -> `main()` -> Resources -> Components -> Messages/Events -> Systems -> Utility functions.

Sections use `// MARK:` comments.

### Bevy ECS Patterns

- **Components**: newtype tuple structs (`Id(usize)`, `Scale(f32)`, `Position(Vec2)`) or unit marker structs (`MyImage`, `MyCursor`, `MyText`). All derive `Component`.
- **Resources**: newtype tuples or named-field structs. All derive `Resource`.
- **Events**: called "Messages" in Bevy 0.18. Use `#[derive(Message)]`. Written via `MessageWriter<T>`, read via `MessageReader<T>`.
- **Entity spawning**: use inline tuple bundles, not custom `Bundle` structs.
- **System registration**: grouped in `add_systems(Update, (...).run_if(in_state(...)))` tuples. Bevy limits anonymous sets to 20 systems, so there are multiple `add_systems` blocks.

### Event-Driven Communication

Systems communicate via buffered events (`MessageWriter<T>` / `MessageReader<T>`), not direct calls. Signal events (no payload) use `is_empty()` / `clear()` pattern; data events iterate with `for ev in reader.read()`. Two-stage image loading pipeline: `LoadNewImageEvent` -> `NewImageLoadedEvent`.

### Configuration

Default config embedded via `include_str!("../assets/default/config.toml")`. User config persisted at `~/.image_viewer` (TOML). Falls back to embedded default if user file missing. Saved with `toml::to_string_pretty`.

## Code Style Guidelines

### Formatting

Configured in `rustfmt.toml`:
- `max_width = 120`
- `tab_spaces = 4`

Always run `cargo fmt` before committing.

### Imports

Two groups separated by blank lines:
1. `std` imports (alphabetical, one per line)
2. External crate imports (alphabetical by crate name)

- Prefer individual imports over nested braces for `std` (`std::fs::canonicalize` and `std::fs::File` on separate lines, not grouped)
- Glob imports only for preludes: `bevy::prelude::*`, `std::io::prelude::*`
- Scoped imports inside function bodies are acceptable for localized use

### Naming Conventions

| Kind | Convention | Examples |
|------|-----------|----------|
| Structs/Enums | PascalCase | `GridLayoutState`, `ScrollBehavior` |
| Bevy disambiguation | `My` prefix | `MyImage`, `MyCursor`, `MyText`, `MyHelp` |
| Event handlers | `on_` prefix | `on_load_image`, `on_move_image`, `on_resize_system` |
| Keyboard dispatchers | `key_` prefix | `key_toggle_cursor`, `key_save_cropped`, `key_change_layout` |
| UI systems | `ui_` prefix | `ui_bottom_menu`, `ui_settings_menu`, `ui_edit_short_cut` |
| Functions | snake_case verbs | `change_global_zoom`, `toggle_cursor`, `scroll_events` |
| Variables | snake_case | `cursor_position`, `scale_factor`, `num_images` |
| Constants | SCREAMING_SNAKE_CASE | `HELP_STRING` |
| Components (newtype) | PascalCase tuple struct | `Id(usize)`, `Scale(f32)`, `Position(Vec2)` |
| Components (marker) | PascalCase unit struct | `MyImage`, `MyCursor` |

### Type Annotations

Rely on Rust type inference. Use explicit annotations only when needed:
- Numeric disambiguation: `2f32`, `0.5f32`, `2_f32`, trailing dot (`1.`, `0.`)
- Turbofish: `toml::to_string_pretty::<Config>(&*config)`
- Deserialization hints: `let Some(config): Option<Config> = ...`
- Never annotate closure parameters

### Error Handling

**Dominant pattern** -- `let-else` with early exit:
```rust
let Some(f) = File::open(&path).ok() else {
    println!("Failed to open file: {}", path);
    continue; // or return
};
```

**For Result types directly:**
```rust
let Ok(ctx) = egui_ctx.ctx_mut() else { return };
```

**Labeled blocks** for multi-step fallible sequences:
```rust
let result = 'block: {
    let Some(x) = step1() else { break 'block None; };
    let Some(y) = step2(x) else { break 'block None; };
    Some(y)
};
```

Rules:
- `?` operator only in `main()` and pure validation utilities
- `.unwrap()` only when architecturally guaranteed safe (e.g., `.single().unwrap()` on Bevy queries known to have exactly one match)
- All error messages use `println!()` (not `eprintln!`), with human-readable descriptions including the problematic value
- No logging framework -- just `println!()`

### Iteration

- Prefer imperative `for` loops over functional iterator chains (`.map().filter().collect()`)
- Use `.iter().count()` for counting query results
- Use `for ev in reader.read() { ... }` for processing events
- Signal events use `is_empty()` / `clear()` pattern instead of iterating

### Comments

- Line comments (`//`) only -- no doc comments (`///`)
- Navigation markers: `// MARK: Section Name`
- Informal, conversational tone
- No comments on obvious code; comment the "why" not the "what"

### Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| bevy | 0.18 | ECS framework, rendering, windowing |
| bevy_egui | 0.39 | Immediate-mode UI integration |
| clap | 4 | CLI argument parsing (derive mode) |
| image | 0.25 | Image format decoding/encoding |
| serde + toml | 1 | Configuration serialization |
| home | 0.5 | Home directory resolution |

### Bevy-Specific Conventions

- System parameters: `Res<T>` (read), `ResMut<T>` (write), `Query<...>`, `Commands`, `Local<T>`
- Use `..default()` (not `Default::default()`) for Bevy struct initialization
- Visibility control: `Visibility::Visible` / `Visibility::Hidden`
- System ordering: `.after()` for explicit dependencies; `run_if(in_state(...))` for state guards
