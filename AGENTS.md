# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Overview

A cross-platform image viewer built with **Bevy 0.18** and Rust. Uses Bevy's Entity-Component-System architecture. Supports comparing images side-by-side, in grids, stacked, or in horizontal/vertical layouts. Includes a review mode for navigating through sets of related images using pattern matching.

## Build & Development Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build (binary: image-viewer)
cargo run -- img1.png img2.png # Run with image arguments
cargo fmt                      # Format code (see rustfmt.toml)
cargo clippy                   # Lint (no clippy.toml; use defaults)
cargo test                     # Run tests (review module only)
cargo bundle --release         # Create macOS app bundle (requires cargo-bundle)
```

## Architecture

### File Structure

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | ~2300 | Application entry point, ECS systems, UI, image pipeline |
| `src/review.rs` | ~400 | Review mode: pattern extraction, directory scanning, file resolution |

### Code Organization (top to bottom in main.rs)

Sections use `// MARK:` comments:

Imports -> Type alias -> CLI args (clap) -> Constants -> App states -> Enums -> Config structs -> `main()` -> Resources -> Components -> Messages/Events -> Setup -> UI systems (egui) -> Image loading pipeline -> Layout & positioning -> Image reordering/removal -> Keyboard handlers -> Zoom/rotation -> Mouse/scroll input -> Image cropping -> Settings -> macOS dock integration -> Review mode systems -> Utilities

### Bevy ECS Patterns

- **Components**: newtype tuple structs (`Id(usize)`, `Scale(f32)`, `Position(Vec2)`) or unit marker structs (`MyImage`, `MyCursor`, `MyText`). All derive `Component`.
- **Resources**: newtype tuples or named-field structs. All derive `Resource`.
- **Events**: called "Messages" in Bevy 0.18. Use `#[derive(Message)]`. Written via `MessageWriter<T>`, read via `MessageReader<T>`.
- **Entity spawning**: use inline tuple bundles, not custom `Bundle` structs.
- **System registration**: grouped in `add_systems(Update, (...).run_if(in_state(...)))` tuples. Bevy limits anonymous sets to 20 systems, so there are multiple `add_systems` blocks. Egui systems use `EguiPrimaryContextPass` schedule instead of `Update`.

### Event-Driven Communication

Systems communicate via buffered events (`MessageWriter<T>` / `MessageReader<T>`), not direct calls. Signal events (no payload) use `is_empty()` / `clear()` pattern; data events iterate with `for ev in reader.read()`.

Key event chains:
- **Image loading**: `LoadNewImageEvent` -> `on_load_image()` decodes image -> `NewImageLoadedEvent` -> `on_image_loaded()` spawns entities -> `on_image_spawned()` triggers `FitToScreen`
- **Layout**: `MoveImageEvent` triggers position/crop recalculation for all images
- **Review navigation**: `NavigateReviewEvent` -> loads new image set; `ActivateReviewEvent` -> extracts patterns from current images; `RefreshReviewEvent` -> rescans directory with edited regexes
- **Image management**: `RemoveImageEvent` -> despawns entities, reassigns contiguous Ids; `ReorderImagesEvent` -> remaps Ids via two-pass rename

### Configuration

Default config embedded via `include_str!("../assets/default/config.toml")`. User config persisted at `~/.image_viewer` (TOML). Falls back to embedded default if user file missing. Saved with `toml::to_string_pretty`.

### Review Module (`src/review.rs`)

Self-contained module for the review feature. Pure functions (no Bevy dependencies):
- `extract_patterns()`: finds common prefix/radix across filenames, derives per-cell regex patterns
- `scan_radixes()`: scans a directory and collects all radixes matching at least 2 cell patterns
- `resolve_files_for_radix()`: resolves concrete file paths for a given radix

Has its own test suite using `tempfile` for directory-based tests.

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
| Structs/Enums | PascalCase | `GridLayoutState`, `ScrollBehavior`, `ReviewState` |
| Bevy disambiguation | `My` prefix | `MyImage`, `MyCursor`, `MyText`, `MyHelp` |
| Event handlers | `on_` prefix | `on_load_image`, `on_move_image`, `on_remove_image`, `on_navigate_review` |
| Keyboard dispatchers | `key_` prefix | `key_toggle_cursor`, `key_save_cropped`, `key_change_layout` |
| UI systems | `ui_` prefix | `ui_bottom_menu`, `ui_settings_menu`, `ui_image_list_panel`, `ui_review_panel` |
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
- `?` operator only in `main()` and pure validation utilities (e.g., `review.rs` helper functions)
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
| regex | 1 | Review mode pattern matching |
| tempfile | 3 | Test-only: temporary directories for review tests |

### Bevy-Specific Conventions

- System parameters: `Res<T>` (read), `ResMut<T>` (write), `Query<...>`, `Commands`, `Local<T>`
- Use `..default()` (not `Default::default()`) for Bevy struct initialization
- Visibility control: `Visibility::Visible` / `Visibility::Hidden`
- System ordering: `.after()` for explicit dependencies; `run_if(in_state(...))` for state guards
- Egui systems run in `EguiPrimaryContextPass` schedule, not `Update`

### macOS-Specific

`macos_dock_drop` module uses `objc2` to inject `application:openURLs:` into Bevy's Winit delegate at runtime via `class_addMethod()`. Paths queued in a static `Mutex<Vec<String>>`, polled by `poll_dock_drop_queue()` system.
