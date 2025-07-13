# E2E Leak Testing Skip Analysis and Implementation Plan

## Executive Summary

This document analyzes the current memory leak testing setup in the webpipe project and provides a plan to exclude end-to-end (e2e) tests from leak detection. The e2e tests spawn external server processes that interfere with memory leak detection tools, making the results unreliable and unnecessarily slow.

## Current Testing Architecture

### Test Categories and Structure

The project has a well-organized three-tier testing structure:

1. **Unit Tests** (`test/unit/`):
   - `test_arena.c` - Memory arena allocator tests
   - `test_lexer.c` - Token parsing tests  
   - `test_parser.c` - AST parsing tests
   - `test_middleware.c` - Core middleware functionality tests

2. **Integration Tests** (`test/integration/`):
   - `test_jq.c` - jq middleware integration with dynamic loading
   - `test_lua.c` - Lua middleware integration
   - `test_mustache.c` - Mustache templating middleware
   - `test_mustache_partials.c` - Mustache partial template tests
   - `test_pg.c` - PostgreSQL database middleware
   - `test_pipeline.c` - Full pipeline execution tests
   - `test_validate.c` - Input validation middleware

3. **System Tests** (`test/system/`):
   - `test_server.c` - Server functionality (in-process testing)
   - `test_e2e.c` - End-to-end tests (external process testing)
   - `test_perf.c` - Performance testing (in-process)

### Current Leak Testing Implementation

**GitHub Actions (`.github/workflows/test.yml`):**
- Both Ubuntu and macOS runners execute `make test-leaks`
- This tests ALL binaries including e2e tests
- Valgrind is used on Linux, native `leaks` tool on macOS

**Makefile:**
```makefile
test-leaks: $(TEST_ALL_BINS)
	./test-runner.sh leaks $(TEST_ALL_BINS)
```
- `TEST_ALL_BINS` includes all unit, integration, and system tests
- No distinction between process-spawning and in-process tests

**test-runner.sh:**
- `run_leaks()` function applies valgrind/leaks to all provided binaries
- Platform detection for Linux (valgrind) vs macOS (leaks tool)
- No filtering mechanism for different test types

## Problem Analysis: Why E2E Tests Break Leak Detection

### test_e2e.c Process Spawning Behavior

The `test_e2e.c` file demonstrates why it's problematic for leak testing:

```c
// Fork a child process to run the server
server_pid = fork();

if (server_pid == 0) {
    // Child process - run the server
    execl("./build/wp", "./build/wp", "test.wp", "--test", "--port", port_arg, NULL);
    exit(1);
} else if (server_pid > 0) {
    // Parent process - wait for server to start
    sleep(2);
    // Make HTTP requests via libcurl to the external server
}
```

**Key Issues:**

1. **Process Boundary Confusion**: Leak detectors track memory within a single process. When `test_e2e` forks and execs the wp server, the parent test process and child server process have separate memory spaces.

2. **False Positives**: The test harness may show "leaks" that are actually:
   - Memory intentionally held by the long-running server process
   - Network buffers in libcurl
   - Database connection pools maintained across requests

3. **Performance Impact**: Running valgrind on process-spawning tests is significantly slower:
   - The parent process runs under valgrind
   - HTTP requests have network latency
   - Database setup/teardown overhead

4. **Unreliable Results**: External processes may not terminate cleanly under valgrind, leading to test flakiness.

### Comparison with Other System Tests

**test_server.c** (should be leak-tested):
```c
// Test server initialization using safe test runtime
int result = init_test_runtime("test.wp");
// Direct in-process API calls, no fork/exec
```

**test_perf.c** (should be leak-tested):
```c
// Direct middleware execution testing
json_t *output = middleware->execute(input, arena, ...);
// Performance measurement of in-process operations
```

## Recommended Solution

### Makefile-Based Granular Leak Testing

Following the same pattern as the existing non-leak test targets, create separate leak testing targets that can be run independently in CI:

```makefile
# New granular leak testing targets
test-leaks-unit: $(TEST_UNIT_BINS)
	./test-runner.sh leaks $(TEST_UNIT_BINS)

test-leaks-integration: $(TEST_INTEGRATION_BINS) 
	./test-runner.sh leaks $(TEST_INTEGRATION_BINS)

test-leaks-system: $(BUILD_DIR)/test_server $(BUILD_DIR)/test_perf
	./test-runner.sh leaks $(BUILD_DIR)/test_server $(BUILD_DIR)/test_perf

# Keep existing target for backward compatibility
test-leaks: $(TEST_ALL_BINS)
	./test-runner.sh leaks $(TEST_ALL_BINS)
```

**Benefits:**
- **Mirrors existing CI pattern**: Just like we run `test-unit`, `test-integration`, and `test-system` separately
- **Selective leak testing**: Only run leak detection where it makes sense
- **Improved CI visibility**: Each leak test category has its own CI step with clear pass/fail status
- **Parallel execution potential**: Different leak test categories could run in parallel
- **Backward compatibility**: Existing `test-leaks` target remains for local development
- **Simple implementation**: Minimal changes to existing infrastructure

## Implementation Plan

### Phase 1: Makefile Updates

1. **Add new granular leak testing targets**:
   - `test-leaks-unit` - Unit tests only
   - `test-leaks-integration` - Integration tests only  
   - `test-leaks-system` - System tests excluding e2e (only `test_server` and `test_perf`)
2. **Keep existing `test-leaks` target** for backward compatibility
3. **Test locally** to ensure all targets work correctly

### Phase 2: GitHub Actions Updates

Update `.github/workflows/test.yml` to run leak tests separately, following the same pattern as the existing non-leak tests:

```yaml
# Replace this single step in both Ubuntu and macOS jobs:
- name: Run leak detection
  run: make test-leaks
  
# With these separate steps:
- name: Run unit leak detection
  run: make test-leaks-unit
  env:
    WP_PG_HOST: localhost
    WP_PG_USER: postgres
    WP_PG_PASSWORD: postgres
    WP_PG_DATABASE: wp-test

- name: Run integration leak detection  
  run: make test-leaks-integration
  env:
    WP_PG_HOST: localhost
    WP_PG_USER: postgres
    WP_PG_PASSWORD: postgres
    WP_PG_DATABASE: wp-test

- name: Run system leak detection
  run: make test-leaks-system
  env:
    WP_PG_HOST: localhost
    WP_PG_USER: postgres
    WP_PG_PASSWORD: postgres
    WP_PG_DATABASE: wp-test
```

**Note**: E2E tests (`test_e2e`) will continue to run in the "Run system tests" step but will be excluded from leak detection.

### Phase 3: Documentation and Validation

1. **Test locally** that each new target works correctly
2. **Validate** that e2e tests still run in regular `test-system` target
3. **Confirm** that leak testing covers all appropriate tests without e2e
4. **Update documentation** if needed

## Expected Outcomes

### Performance Improvements

- **Faster CI builds**: Removing valgrind from e2e tests will significantly reduce build times
- **More reliable tests**: Fewer false positives and test failures due to external process issues

### Better Test Coverage

- **Focused leak detection**: Memory leak testing will focus on code that can actually be analyzed
- **Maintained functionality testing**: E2E tests continue to run in regular test suites

### Cleaner Architecture

- **Logical separation**: Clear distinction between different types of testing
- **Future extensibility**: Easy to add more test categories or exclusion rules

## Risk Assessment

### Low Risk Changes

- Makefile modifications are additive and maintain backward compatibility
- GitHub Actions changes are minimal and easily reversible
- No changes to actual test code

### Mitigation Strategies

- Keep existing `test-leaks` target for manual testing if needed
- Test changes thoroughly in CI before merging
- Document the new testing approach clearly

## Alternative Approaches Considered

### Single Combined Leak Target with Exclusions
**Rejected**: Would require modifying test-runner.sh with filtering logic, adding complexity.

### Environment Variable-Based Conditional Testing
**Rejected**: Adding environment variable logic increases complexity without clear benefits.

### Mocking External Processes in E2E Tests
**Rejected**: Would require significant test refactoring and lose end-to-end validation.

### Static Analysis Only
**Rejected**: Runtime leak detection provides valuable insights that static analysis cannot capture.

## Conclusion

The recommended approach of creating granular Makefile leak testing targets provides a clean, maintainable solution that:

- **Mirrors existing CI patterns**: Uses the same separate-step approach as non-leak tests
- **Improves CI performance and reliability**: Eliminates problematic e2e leak testing
- **Maintains comprehensive test coverage**: E2E tests continue to run, just without leak detection
- **Preserves backward compatibility**: Existing `test-leaks` target remains available
- **Provides better CI visibility**: Each leak test category has its own step with clear pass/fail status
- **Follows established project patterns**: Consistent with existing test organization

This solution addresses the core issue of unreliable e2e leak testing while improving the overall CI architecture and maintainability.