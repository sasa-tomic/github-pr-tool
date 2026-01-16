# Code Smell Fixes TODO

## High Priority

### 1. Double GPT API Call (MAJOR - $$, latency)
- **Location**: `src/main.rs` lines 329-338 and 393-401
- **Problem**: `gpt_generate_branch_name_and_commit_description` is called twice - once for uncommitted changes and once for branch diff. The branch name from the second call is discarded.
- **Fix**: Restructure to call GPT once with the appropriate diff (branch diff when available, uncommitted otherwise)
- **Status**: [x] COMPLETED
- **Solution**: Added `cached_gpt_response` to store the first GPT response when creating a fresh branch from main. When the branch is freshly created, the PR reuses the cached response instead of making a second API call. Only calls GPT twice when updating an existing branch with prior commit history (where the full branch diff differs from uncommitted changes).

### 2. Useless Tokio Task (MEDIUM - confusion)
- **Location**: `src/main.rs` lines 221-231
- **Problem**: Spawns a task that just sleeps and updates a local variable, never used before being aborted
- **Fix**: Remove entirely
- **Status**: [ ] Not started

### 3. Excessive UI Boilerplate (HIGH - readability)
- **Location**: `src/main.rs` throughout `run()` function
- **Problem**: `terminal.draw()` and `check_events()` called 30+ times, obscuring actual logic
- **Fix**: Create a helper macro or wrapper, or redesign UI update architecture
- **Status**: [ ] Not started

### 4. Convoluted Cleanup Logic (HIGH - bugs)
- **Location**: `src/main.rs` lines 461-515
- **Problem**: `ui_update.abort()` called twice, `run_event_loop` conditionally called in multiple places, explicit `std::mem::drop`, confusing branching
- **Fix**: Redesign with clearer state machine or RAII pattern
- **Status**: [ ] Not started

## Medium Priority

### 5. Duplicate Parent Branch Detection (MEDIUM)
- **Location**: `src/git_ops.rs` lines 159-217 in `git_diff_between_branches`
- **Problem**: Same parent branch detection logic copy-pasted twice
- **Fix**: Extract to a single helper function
- **Status**: [ ] Not started

### 6. Duplicate Patch Creation Logic (MEDIUM)
- **Location**: `src/git_ops.rs` (git_fetch_main) and `src/git_temp_worktree.rs` (TempWorktree::enter)
- **Problem**: Patch creation happens in two places with `try_reuse_recent_patches` as a workaround
- **Fix**: Centralize patch creation in one module
- **Status**: [ ] Not started

### 7. Duplicate GitHub Issues Fetch (LOW)
- **Location**: `src/main.rs` lines 323 and 391
- **Problem**: `github_list_issues` called twice (cache mitigates impact but code is confusing)
- **Fix**: Fetch once and reuse the variable
- **Status**: [ ] Not started

## Completed

### 8. Flaky Integration Tests (FIXED)
- **Location**: `tests/git_ops_tests.rs`
- **Problem**: Tests used `env::set_current_dir()` which is global state - parallel tests interfered with each other
- **Fix**: Added `serial_test` dependency and marked tests with `#[serial]`
- **Status**: [x] COMPLETED

### 9. Clippy Warning (FIXED)
- **Location**: `src/tui.rs` line 127
- **Problem**: Manual implementation of `.is_multiple_of()` - used `% 2 == 0` instead
- **Fix**: Changed to `app.blink_timer.is_multiple_of(2)`
- **Status**: [x] COMPLETED
