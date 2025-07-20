# R Middleware Implementation Plan

## Overview
Complete the R middleware implementation for the WebPipe framework based on existing architecture patterns and R embedding capabilities.

## Current Status Analysis

### What's Already Implemented
- Basic R initialization with `Rf_initEmbeddedR()`
- Expression caching system with hash table (64 entries)
- JSON ↔ R conversion functions (`json_to_sexp`, `sexp_to_json`)
- Thread safety with pthread mutex
- Integration with jsonlite package for advanced JSON handling
- Basic error handling for parse and evaluation failures
- Proper cleanup with destructor function

### Architecture Patterns (from other middleware)

#### JQ Middleware Pattern
- Thread-safe caching system with per-entry mutexes
- Arena-aware string allocation
- Conversion functions with depth limiting
- Error handling with standardized format

#### Lua Middleware Pattern  
- Database API integration (`executeSql` function)
- Per-request state creation (new lua_State per request)
- Arena integration for memory management
- Conversion functions with depth protection

#### PostgreSQL Middleware Pattern
- `middleware_init()` for startup configuration
- Database provider registration via `execute_sql()` function
- Configuration parsing from JSON config blocks
- Standardized error format with detailed diagnostics

## Implementation Plan

### Phase 1: Complete Core Functionality ✅
**Status: Already Implemented**
- ✅ R initialization and teardown
- ✅ Expression parsing and caching  
- ✅ JSON ↔ R conversion
- ✅ Thread safety with mutex
- ✅ Basic middleware interface

### Phase 2: Enhance Error Handling
**Current Issues:**
- Error messages are basic (`"R parse error"`, `"R evaluation error"`)
- No standardized error format like other middleware
- No detailed R error information capture

**Improvements Needed:**
1. Implement standardized error format with `errors` array
2. Capture detailed R error messages using R's error handling
3. Add error context (line numbers, expression details)
4. Handle R warnings and messages

### Phase 3: Arena Integration
**Current Issues:**
- Arena parameters are ignored (`(void)arena; (void)alloc; (void)free_`)
- No memory management integration with request arena
- String allocations may use global malloc

**Improvements Needed:**
1. Integrate arena allocator for R string conversions
2. Use arena for cached expression keys
3. Ensure all per-request allocations use arena

### Phase 4: Database Integration (Optional)
**Following Lua Pattern:**
1. Add database API access if available
2. Provide R functions for SQL execution
3. Enable `executeSql()` equivalent in R environment

### Phase 5: Configuration Support
**Following PostgreSQL Pattern:**
1. Implement `middleware_init()` function
2. Support R-specific configuration options:
   - R_HOME path override
   - Package loading preferences  
   - Memory limits
   - Security restrictions
3. Parse JSON config blocks

### Phase 6: Advanced Features
1. **Content Type Support**: Allow R to return different content types (HTML, SVG, etc.)
2. **Package Management**: Auto-load commonly used packages
3. **Security**: Restrict dangerous R functions
4. **Performance**: Optimize conversion functions

## Implementation Details

### Standardized Error Format
```c
// Replace current error handling:
return json_pack("{s:s}", "error","R parse error");

// With standardized format:
json_t *error_obj = json_object();
json_t *errors_array = json_array();
json_t *error_detail = json_object();

json_object_set_new(error_detail, "type", json_string("rError"));
json_object_set_new(error_detail, "message", json_string(detailed_error_msg));
json_object_set_new(error_detail, "expression", json_string(filter));

json_array_append_new(errors_array, error_detail);
json_object_set_new(error_obj, "errors", errors_array);
```

### Arena Integration
```c
// For string allocations in conversion functions:
if (arena && alloc_func) {
    char *arena_str = alloc_func(arena, len + 1);
    // Use arena_str instead of malloc
}
```

### Configuration Support
```c
int middleware_init(json_t *config) {
    // Parse R-specific configuration
    const char *r_home = get_config_string(config, "r_home");
    if (r_home) {
        setenv("R_HOME", r_home, 1);
    }
    
    // Initialize R with configuration
    return r_middleware_init(config);
}
```

## Testing Strategy

### Demo Route for test.wp
```wp
# R Middleware Demo Routes
GET /r/hello
  |> r: `list(message = "Hello from R!", timestamp = Sys.time())`

GET /r/math
  |> jq: `{ numbers: [1, 2, 3, 4, 5] }`
  |> r: `list(
    input = .input$numbers,
    sum = sum(.input$numbers),
    mean = mean(.input$numbers),
    stats = summary(.input$numbers)
  )`

GET /r/plot-data
  |> jq: `{ x: [1,2,3,4,5], y: [2,4,6,8,10] }`
  |> r: `
    df <- data.frame(x = .input$x, y = .input$y)
    model <- lm(y ~ x, data = df)
    list(
      data = df,
      slope = coef(model)[2],
      intercept = coef(model)[1],
      r_squared = summary(model)$r.squared
    )
  `

GET /r/json-output
  |> jq: `{ name: "Alice", age: 30 }`
  |> r: `
    # Using jsonlite for complex output
    result <- list(
      greeting = paste("Hello", .input$name),
      info = list(
        age_group = if(.input$age >= 30) "adult" else "young",
        processed_at = Sys.time()
      )
    )
    jsonlite::toJSON(result, auto_unbox = TRUE)
  `
```

## Completion Criteria

### Phase 2 (Error Handling) - HIGH PRIORITY
- [ ] Standardized error format implementation
- [ ] Detailed R error message capture  
- [ ] R warning and message handling
- [ ] Error context information

### Phase 3 (Arena Integration) - MEDIUM PRIORITY  
- [ ] Arena-based string allocations
- [ ] Memory leak prevention
- [ ] Per-request memory management

### Phase 4 (Database Integration) - LOW PRIORITY
- [ ] Database API integration (if needed)
- [ ] R-side SQL execution functions

### Phase 5 (Configuration) - MEDIUM PRIORITY
- [ ] `middleware_init()` implementation
- [ ] JSON config block parsing
- [ ] R environment configuration

### Phase 6 (Advanced Features) - LOW PRIORITY
- [ ] Content type support
- [ ] Package management
- [ ] Security restrictions
- [ ] Performance optimizations

## Files to Modify

1. **src/middleware/r.c** - Main implementation
2. **test.wp** - Add demo routes  
3. **Makefile** - Ensure R linking works on macOS
4. **README.md** - Update R middleware documentation

## Build Requirements (from r-demo.txt)
- macOS: R framework at `/Library/Frameworks/R.framework/Resources`
- Compiler flags: `-I$(R_HOME)/include -std=gnu99 -DDEFAULT_R_HOME=...`
- Linker flags: `-F/Library/Frameworks -framework R`
- Auto-install tidyverse if needed

The current implementation is already quite solid and follows the WebPipe patterns well. The main improvements needed are standardized error handling and arena integration to match the quality and consistency of other middleware.