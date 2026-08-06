```markdown
# niri Development Patterns

> Auto-generated skill from repository analysis

## Overview

This skill teaches you how to contribute effectively to the **niri** codebase, a Rust project that appears to be a window manager or similar system-level application. You'll learn about its coding conventions, common workflows for implementing features, fixing bugs, updating documentation, managing dependencies, and handling Nix build configurations. This guide covers file organization, commit patterns, and practical step-by-step instructions for each type of contribution.

## Coding Conventions

- **File Naming:**  
  Use `camelCase` for file names.  
  _Example:_  
  ```
  src/input/inputHandler.rs
  niri-config/src/userConfig.rs
  ```

- **Import Style:**  
  Use **relative imports** within modules.  
  _Example:_  
  ```rust
  use super::inputHandler;
  use crate::ui::overlay;
  ```

- **Export Style:**  
  Use **named exports**.  
  _Example:_  
  ```rust
  pub struct InputEvent { /* ... */ }
  pub fn handle_input() { /* ... */ }
  ```

- **Commit Patterns:**  
  - Prefixes: `fix`, `input`, `nix`, `wiki`, `feat`, `render`, `layout`, `pw_utils`
  - Messages are short (~43 characters), e.g.:  
    ```
    fix: resolve crash when switching workspaces
    feat: add support for multi-touch input
    ```

## Workflows

### Feature Implementation with Config and Docs
**Trigger:** When adding a new user-facing, configurable feature  
**Command:** `/new-configurable-feature`

1. **Update Configuration:**  
   Add or modify configuration in `niri-config/src/*.rs`.
   ```rust
   // niri-config/src/userConfig.rs
   pub struct UserConfig {
       pub enable_cool_feature: bool,
       // ...
   }
   ```
2. **Implement Feature:**  
   Update or add code in `src/**/*.rs` to implement the feature.
   ```rust
   // src/features/coolFeature.rs
   pub fn activate_cool_feature() { /* ... */ }
   ```
3. **Document:**  
   Update or add documentation in `docs/wiki/*.md`.
   ```
   ## Cool Feature
   Enable this in your config to use the new feature.
   ```

---

### Input Feature or Bugfix
**Trigger:** When adding or fixing input device support or behavior  
**Command:** `/input-feature`

1. **Modify Input Logic:**  
   Edit `src/input/mod.rs` to implement or fix input handling.
   ```rust
   // src/input/mod.rs
   pub fn handle_new_input_device() { /* ... */ }
   ```
2. **Update Configuration (Optional):**  
   Change `niri-config/src/*.rs` if new config options are needed.
3. **Update UI Overlays (Optional):**  
   Edit `src/ui/*.rs` for any UI changes related to input.

---

### Documentation Update
**Trigger:** When clarifying, adding, or fixing documentation  
**Command:** `/update-docs`

1. **Edit Documentation:**  
   Update or add `docs/wiki/*.md` files.
2. **Update README (Optional):**  
   Edit `README.md` if necessary.

---

### Nix or Flake Configuration Update
**Trigger:** When changing build inputs, dependencies, or build environment  
**Command:** `/update-nix`

1. **Edit Nix Files:**  
   Update `flake.nix` and/or `flake.lock`.
2. **Update Build Scripts (Optional):**  
   Modify related scripts if needed.

---

### Dependency Upgrade
**Trigger:** When updating a third-party Rust crate  
**Command:** `/upgrade-dependency`

1. **Update Cargo Files:**  
   Edit `Cargo.toml` and `Cargo.lock` to bump the dependency version.
2. **Adjust Code:**  
   Update `src/**/*.rs` to accommodate API changes or new features from the upgraded crate.

   _Example:_  
   ```toml
   # Cargo.toml
   [dependencies]
   cool-crate = "2.0"
   ```
   ```rust
   // src/lib.rs
   use cool_crate::new_api;
   ```

## Testing Patterns

- **Test File Naming:**  
  Test files use the `*.test.*` pattern, e.g., `inputHandler.test.rs`.
- **Framework:**  
  The specific test framework is not detected, but standard Rust testing (`#[cfg(test)]`, `#[test]`) is likely used.
- **Example:**  
  ```rust
  // src/input/inputHandler.test.rs
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn test_input_event() {
          // test logic here
      }
  }
  ```

## Commands

| Command                   | Purpose                                                      |
|---------------------------|--------------------------------------------------------------|
| /new-configurable-feature | Start a new configurable feature with config and docs         |
| /input-feature            | Add or fix an input-related feature or bug                   |
| /update-docs              | Update or clarify documentation                              |
| /update-nix               | Update Nix or flake build configuration                      |
| /upgrade-dependency       | Upgrade a Rust crate dependency and update code as needed    |
```