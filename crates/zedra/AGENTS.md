# zedra

Main mobile app crate. Owns GPUI application flow, workspace orchestration, platform bridge integration, and all product UI above the reusable terminal emulator.

## What This Crate Owns

- app bootstrap and screen routing in `src/app.rs`
- workspace orchestration in `src/workspace.rs` and `src/workspaces.rs`
- persisted display state in `src/workspace_state.rs`
- platform abstraction in `src/platform_bridge.rs`
- iOS and Android native bridge code under `src/ios/` and `src/android/`
- product UI components, drawers, sheets, panels, editor, and terminal integration

## Relationship To vendor/zed

- `vendor/zed` is a submodule we actively patch for mobile GPUI support. Do not treat it as untouchable upstream code.
- For GPUI behavior, platform integration, grammars, and interaction details, inspect the corresponding code in `vendor/zed` before inventing new patterns in `zedra`.
- For editor work, Zed desktop is reference material, not a direct template. Zedra editor views should stay minimal, mobile-specific, and pragmatic even when borrowing ideas from desktop implementations.

## Core UI Rules

- `WorkspaceState` remains the display source of truth. Views should read workspace display fields from `WorkspaceState`, not from `SessionHandle` directly during render.
- Keep `render()` pure. Mutations belong in event handlers, subscriptions, `cx.spawn`, `cx.spawn_in`, or platform callbacks.
- `Workspace` is the orchestrator between `Session`, `SessionState`, `WorkspaceState`, terminal entities, and action handling. Avoid pushing app-level orchestration down into leaf views.
- `Workspaces` manages the list of active workspace entities and saved workspace state. Preserve the distinction between persisted state and live workspace entries.

## GPUI Patterns In This Crate

- Prefer entity/subscription wiring over ad hoc shared mutable state.
- Use events for child-to-parent communication when the parent owns the entity reference.
- Use actions for decoupled commands that bubble through the view tree.
- Preserve the drawer and sheet interaction patterns already established in `ui/drawer_host.rs`, `sheet_host_view.rs`, and `native_presentation.rs`.
- Scroll containers still need explicit `.id(...)` plus constrained parent height. Follow the existing `size_full()` and `min_h_0()` patterns when editing nested layouts.

## Platform Rules

- Always use `platform_bridge::bridge()` for platform-specific behavior from shared UI code.
- Native UIKit and Android integration belongs behind the platform bridge or the native platform modules, not inside general GPUI views.
- iOS custom sheet hosting relies on the ownership split in `src/ios/app.rs` and `src/platform_bridge.rs`. Preserve that flow instead of inventing a parallel presentation path.
- Android app lifecycle and surface management are main-thread-sensitive. Be conservative when changing `src/android/app.rs`, `src/android/jni.rs`, or the command queue.

## Workspace And Terminal Integration

- `WorkspaceTerminal` is the seam between app-level terminal management and `zedra-terminal`. Keep terminal attach, resize, title updates, and active-input registration aligned there.
- If you change connection or sync behavior, verify the interaction among `Workspace`, `Workspaces`, `SessionState`, and `WorkspaceState::sync_from_session`.
- Saved workspace persistence is intentionally lightweight JSON under the app data directory. Do not mix transient UI state into the persisted model.

## Logging And Manual Validation

- Use `tracing` for lifecycle and debugging logs. For mobile debugging, prefer targeted logs that a developer can grep from device output.
- UI and platform changes should add or update manual verification steps in `docs/MANUAL_TEST.md`.
- If a change affects deep links, keyboard behavior, terminal focus, sheets, or drawer gestures, call that out explicitly in manual test steps.

## Good Change Shapes

- Connection-flow or workspace-flow changes usually touch `workspace.rs`, `workspaces.rs`, and `workspace_state.rs` together.
- Platform bridge additions typically require updates in shared bridge traits plus the corresponding iOS and Android implementations.
- UI changes should preserve the repo’s existing visual and interaction patterns from `docs/DESIGN.md`.
- Editor changes should usually inspect both `src/editor/` and the relevant desktop/reference code under `vendor/zed`, then implement the smallest mobile-appropriate version in Zedra.

## Validation

- `cargo check -p zedra`
- For UI/platform work, add or update `docs/MANUAL_TEST.md`

## Key Files

- `src/app.rs` — root app entity, screen routing, background tick
- `src/workspace.rs` — session/UI orchestration for a live workspace
- `src/workspaces.rs` — saved-state list and active workspace switching
- `src/workspace_state.rs` — persisted display model
- `src/platform_bridge.rs` — shared platform abstraction boundary
- `src/ios/app.rs` and `src/android/app.rs` — native app bootstrap and surface lifecycle
