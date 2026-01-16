# Code Smell Fixes TODO

## Remaining

### 4. Convoluted Cleanup Logic (HIGH - bugs)
- **Location**: `src/main.rs` lines 461-515
- **Problem**: `run_event_loop` conditionally called in multiple places, explicit `std::mem::drop`, confusing branching
- **Fix**: Redesign with clearer state machine or RAII pattern
- **Status**: [ ] Not started

### 6. Duplicate Patch Creation Logic (MEDIUM)
- **Location**: `src/git_ops.rs` (git_fetch_main) and `src/git_temp_worktree.rs` (TempWorktree::enter)
- **Problem**: Patch creation happens in two places with `try_reuse_recent_patches` as a workaround
- **Fix**: Centralize patch creation in one module
- **Status**: [ ] Not started

## Completed

### 1. Double GPT API Call (MAJOR - $$, latency)
- **Location**: `src/main.rs`
- **Problem**: `gpt_generate_branch_name_and_commit_description` was called twice
- **Fix**: Added `cached_gpt_response` to reuse first GPT response for fresh branches
- **Status**: [x] COMPLETED

### 2. Useless Tokio Task (MEDIUM - confusion)
- **Location**: `src/main.rs` lines 221-231
- **Problem**: Spawned a task that just slept and updated a local variable, never used before being aborted
- **Fix**: Removed entirely along with all `ui_update.abort()` calls
- **Status**: [x] COMPLETED

### 3. Excessive UI Boilerplate (HIGH - readability)
- **Location**: `src/main.rs` throughout `run()` function
- **Problem**: `terminal.draw()` and `check_events()` called 30+ times, obscuring actual logic
- **Fix**: Renamed `check_events` to `refresh_ui`, removed redundant `terminal.draw()` calls (reduced from 39 to 10)
- **Status**: [x] COMPLETED

### 5. Duplicate Parent Branch Detection (MEDIUM)
- **Location**: `src/git_ops.rs` in `git_diff_between_branches`
- **Problem**: Same parent branch detection logic copy-pasted twice
- **Fix**: Extracted to `try_find_git_parent_branch()` helper function
- **Status**: [x] COMPLETED

### 7. Duplicate GitHub Issues Fetch (LOW)
- **Location**: `src/main.rs`
- **Problem**: `github_list_issues` called twice
- **Fix**: Moved fetch before main logic, reuse single `issues_json` variable throughout
- **Status**: [x] COMPLETED

### 8. Flaky Integration Tests (FIXED)
- **Location**: `tests/git_ops_tests.rs`
- **Problem**: Tests used `env::set_current_dir()` which is global state - parallel tests interfered
- **Fix**: Added `serial_test` dependency and marked tests with `#[serial]`
- **Status**: [x] COMPLETED

### 9. Clippy Warning (FIXED)
- **Location**: `src/tui.rs` line 127
- **Problem**: Manual implementation of `.is_multiple_of()`
- **Fix**: Changed to `app.blink_timer.is_multiple_of(2)`
- **Status**: [x] COMPLETED
