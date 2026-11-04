# Engineering Rules for AI Agents (`AGENTS.md`)

Every AI coding agent (Claude Code, Codex, Gemini, Cursor, Kimi, or any other agent) working in this repository **must** read and obey these rules before modifying code, running tests, or declaring a task complete.

---

## 1. General Principles & Anti-Shortcut Rules
- Prioritize correctness, complete end-to-end wiring, and maintainability over merely making the build or test suite pass.
- **No Unwired Code (Dead Integration):** A module or function that passes isolated unit tests but is not wired into the actual application entry point (`gui/crates/nopeek-gui/src/main.rs` or `inference/src/no_peek/main.py`), event loop, or IPC pipeline is **not** implemented.
- **No Stubs or Placeholders:** Never leave `todo!()`, `unimplemented!()`, `pass`, dummy returns, hardcoded values, or no-op functions in production paths to satisfy the compiler.
- **No Compiler Silencing:** Do not add `#[allow(dead_code)]`, `#[allow(unused_variables)]`, or `#[allow(clippy::all)]` to hide incomplete wiring. Fix the underlying missing integration instead.
- **No Fake Cross-Platform Stubs:** Never place empty or no-op implementations inside `#[cfg(windows)]` or `#[cfg(target_os = "macos")]` just to make one platform compile. Both platforms must follow the `../oyenza-studio` reference implementation (`crates/poni` and `crates/poni-core`).
- Never remove, disable, or bypass existing functionality just to resolve compilation errors.
- Do not silently introduce architectural changes or deviate from the `../oyenza-studio` native GUI architecture.
- **Project-Specific Visual & Performance Invariants:**
  - **Strictly NO Unicode Emojis:** Never use Unicode emoji characters in UI text, IPC messages, logs, or documentation. Use vector SVG paths (`24x24` grid in `nopeek-core::icons`) exclusively.
  - **Zero Idle CPU/GPU:** The frame clock (VBlank on Windows / `CADisplayLink` on macOS) must park/pause whenever `island.settled() == true`. Never poll in a busy loop.

---

## 2. Error Handling (Rust & Python)
- **No Silent Error Swallowing:**
  - In Rust: Never suppress errors using `unwrap_or_default()`, `.ok()`, or `let _ = ...` on fallible I/O, IPC, or OS calls without an explicit, documented justification.
  - In Python: Never use bare `except:` or `except Exception: pass` to hide runtime, camera, or model failures.
- **No Unjustified Panics:** Do not use `unwrap()` or `expect()` in production paths unless the invariant is mathematically or structurally guaranteed and documented in a comment.
- Never convert operational failures (e.g., broken pipe, missing model, camera disconnect) into successful empty results (`Ok(())`, `None`, or `[]`). Propagate meaningful error context up to the supervisor/logger.

---

## 3. Dependencies
- Do not replace or add dependencies merely because existing APIs (such as `windows` Win32/Direct2D or `objc2` AppKit/CoreGraphics) are verbose or inconvenient.
- Prefer existing workspace dependencies from `../oyenza-studio` and standard library primitives.
- Explain and justify why any new dependency is strictly necessary before adding it.

---

## 4. Testing & Anti-Test-Gaming
- **No Test Gaming:** Never add `cfg!(test)` branches or environment checks inside production logic to return special values just to make tests pass.
- **No Weakening or Skipping Tests:** Never delete, weaken assertions, loosen tolerances, or add `#[ignore]` / `@unittest.skip` to existing tests to make the test suite green.
- **No Tautological Mock Tests:** Do not write tests that only assert the behavior of mocks you created in the same test. Tests must verify real parsing, state transitions, math, and integration contracts.
- Run formatting, `cargo clippy`, `cargo check`, `cargo test`, and Python `unittest` after implementation:
  - Python: `.\.venv\Scripts\python.exe -m unittest discover -s inference/tests -v`
  - Rust: `cargo test --manifest-path gui/Cargo.toml` and `cargo clippy --manifest-path gui/Cargo.toml`
- Explicitly report any tests (e.g., macOS-only `objc2` runtime tests when running on Windows, or vice versa) that could not be executed on the current host OS.

---

## 5. Completion Report & Honesty Contract
Before declaring a task complete, you must provide a structured report containing:
1. **Implemented Functionality:** What works end-to-end vs. what is only scaffolded.
2. **Files Changed:** Full list of created or modified files.
3. **Tests Executed & Results:** Exact commands run and pass/fail counts.
4. **Known Limitations:** Platform constraints or unverified edge cases.
5. **Technical Debt / Remaining Work:** Any incomplete modules, unimplemented stages, or deferred tasks.

**Strict Honesty Rule:** Never claim a feature or application is complete if it is only partially implemented, scaffolded, or only tested in isolation without end-to-end wiring.
