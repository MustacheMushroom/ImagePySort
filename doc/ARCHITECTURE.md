# MediaSift architecture

MediaSift separates testable media operations from the native desktop shell.
The executable entrypoint contains no application behavior; it delegates to the
library crate so desktop state and UI modules can be compiled and tested through
the normal library boundary.

```mermaid
flowchart TD
    main["src/main.rs<br/>Windows entrypoint"] --> desktop["desktop/app.rs<br/>eframe bootstrap"]
    desktop --> state["desktop/state.rs<br/>Application state and selection rules"]
    desktop --> shell["desktop/ui/mod.rs<br/>Responsive shell and theme"]
    shell --> pages["desktop/ui/pages/*.rs<br/>One module per workflow"]
    shell --> dialogs["desktop/ui/dialogs.rs<br/>Confirmations"]
    pages --> components["desktop/ui/components.rs<br/>Reusable widgets and design tokens"]
    dialogs --> components
    pages --> operations["desktop/operations.rs<br/>Background task orchestration"]
    dialogs --> operations
    operations --> core["src/lib.rs<br/>Media and filesystem operations"]
    state --> core
```

## Module responsibilities

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | Select the Windows GUI subsystem and call `desktop::run`. |
| `src/desktop/app.rs` | Configure the native window and construct the application. |
| `src/desktop/state.rs` | Own UI state, typed work results, notices, and pure duplicate-selection rules. |
| `src/desktop/operations.rs` | Open native dialogs, start background work, collect typed results, and update state on the UI thread. |
| `src/desktop/ui/mod.rs` | Render global navigation/status chrome, handle shortcuts, configure themes, and dispatch pages. |
| `src/desktop/ui/pages/` | Render each user workflow in its own module without owning filesystem implementation. |
| `src/desktop/ui/dialogs.rs` | Render confirmations for file-changing actions. |
| `src/desktop/ui/components.rs` | Define semantic design tokens and reusable stateless widgets. |
| `src/desktop/ui/tests.rs` | Exercise responsive page rendering in light and dark themes. |
| `src/lib.rs` | Implement testable media discovery, hashing, sorting, renaming, enhancement, archive, recycle, and deletion behavior. |

## Dependency rules

- UI modules may read and update `MediaSiftApp`, but filesystem work belongs in
  `operations.rs` or the core library.
- Long-running work must stay off the eframe event loop and communicate through
  typed `WorkResult` messages.
- Reusable visual patterns and semantic colors belong in `components.rs`, not in
  individual workflow pages.
- Pure selection or validation behavior belongs in `state.rs` and requires unit
  tests. Rendering behavior belongs in `ui/tests.rs`.
- A new top-level workflow should add a `Page` variant, a page renderer, and one
  navigation entry. It should not add behavior to `src/main.rs`.
