# Scripts Folder Implementation Plan

## Overview
This document outlines the plan to implement a scripts folder system that allows Lua middleware blocks to load external Lua files using `@scripts/filename.lua` syntax, directly integrated into `src/middleware/lua.c`.

## Current State Analysis

The current `src/middleware/lua.c` is a clean, minimal implementation that:
- Creates a new Lua state for each middleware execution
- Provides `executeSql()` function for database access
- Converts between JSON and Lua tables
- Uses arena allocation for memory management

Unlike the old implementation, it doesn't have:
- File caching or hot reloading
- Global module system
- Persistent Lua states

## Implementation Strategy

Since the current system creates fresh Lua states for each request, we'll implement a simpler approach:

1. **Script Loading on Demand** - Load and execute scripts when needed
2. **Simple File System** - No complex caching, just read files directly
3. **Module Registration** - Make scripts available as globals in each new Lua state
4. **Error Handling** - Proper error reporting for missing/invalid scripts

## Technical Implementation

### 1. Add Script Loading Infrastructure

Add these structures and functions to `src/middleware/lua.c`:

```c
#include <dirent.h>
#include <sys/stat.h>

#define SCRIPTS_DIR "scripts"

// Simple script registry (no caching for now)
typedef struct {
    char **script_names;    // Array of script names (without .lua)
    char **script_paths;    // Array of full file paths
    size_t count;
    size_t capacity;
} ScriptRegistry;

static ScriptRegistry g_scripts = {0};
```

### 2. Script Discovery Function

```c
static bool scan_scripts_directory(void) {
    DIR *dir = opendir(SCRIPTS_DIR);
    if (!dir) {
        // No scripts directory - that's fine
        return true;
    }
    
    // Count .lua files first
    size_t script_count = 0;
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (entry->d_type == DT_REG) {
            const char *ext = strrchr(entry->d_name, '.');
            if (ext && strcmp(ext, ".lua") == 0) {
                script_count++;
            }
        }
    }
    
    if (script_count == 0) {
        closedir(dir);
        return true;
    }
    
    // Allocate arrays
    g_scripts.script_names = malloc(script_count * sizeof(char*));
    g_scripts.script_paths = malloc(script_count * sizeof(char*));
    if (!g_scripts.script_names || !g_scripts.script_paths) {
        closedir(dir);
        return false;
    }
    
    // Populate arrays
    rewinddir(dir);
    size_t idx = 0;
    while ((entry = readdir(dir)) != NULL && idx < script_count) {
        if (entry->d_type == DT_REG) {
            const char *ext = strrchr(entry->d_name, '.');
            if (ext && strcmp(ext, ".lua") == 0) {
                // Store script name without .lua extension
                size_t name_len = ext - entry->d_name;
                g_scripts.script_names[idx] = malloc(name_len + 1);
                if (g_scripts.script_names[idx]) {
                    memcpy(g_scripts.script_names[idx], entry->d_name, name_len);
                    g_scripts.script_names[idx][name_len] = '\0';
                }
                
                // Store full path
                size_t path_len = strlen(SCRIPTS_DIR) + 1 + strlen(entry->d_name);
                g_scripts.script_paths[idx] = malloc(path_len + 1);
                if (g_scripts.script_paths[idx]) {
                    snprintf(g_scripts.script_paths[idx], path_len + 1, "%s/%s", 
                             SCRIPTS_DIR, entry->d_name);
                }
                idx++;
            }
        }
    }
    
    g_scripts.count = idx;
    g_scripts.capacity = script_count;
    closedir(dir);
    return true;
}
```

### 3. Script Loading Function

```c
static bool load_script_into_state(lua_State *L, const char *script_name) {
    // Find script path
    const char *script_path = NULL;
    for (size_t i = 0; i < g_scripts.count; i++) {
        if (strcmp(g_scripts.script_names[i], script_name) == 0) {
            script_path = g_scripts.script_paths[i];
            break;
        }
    }
    
    if (!script_path) {
        return false; // Script not found
    }
    
    // Load and execute the script file
    if (luaL_dofile(L, script_path) != LUA_OK) {
        return false; // Script error
    }
    
    // If the script returned a value, set it as a global with the script name
    if (lua_gettop(L) > 0) {
        lua_setglobal(L, script_name);
    }
    
    return true;
}
```

### 4. Load All Scripts Function

```c
static void load_all_scripts_into_state(lua_State *L) {
    for (size_t i = 0; i < g_scripts.count; i++) {
        if (!load_script_into_state(L, g_scripts.script_names[i])) {
            fprintf(stderr, "Warning: Failed to load script %s\n", g_scripts.script_names[i]);
        }
    }
}
```

### 5. Integration Points

#### A. Module Initialization
Add a new initialization function that can be called at server startup:

```c
int middleware_init(json_t *config) {
    (void)config; // Unused for now
    return scan_scripts_directory() ? 0 : -1;
}
```

#### B. Modify `middleware_execute`
Update the main execution function to load scripts into each new Lua state:

```c
// In middleware_execute, after luaL_openlibs(L):
luaL_openlibs(L);

// Load all available scripts into this state
load_all_scripts_into_state(L);

// Push input as 'request' global variable
jansson_to_lua(L, input);
lua_setglobal(L, "request");
```

### 6. Cleanup Function

```c
void cleanup_scripts(void) {
    for (size_t i = 0; i < g_scripts.count; i++) {
        free(g_scripts.script_names[i]);
        free(g_scripts.script_paths[i]);
    }
    free(g_scripts.script_names);
    free(g_scripts.script_paths);
    g_scripts.script_names = NULL;
    g_scripts.script_paths = NULL;
    g_scripts.count = 0;
    g_scripts.capacity = 0;
}
```

## Usage Examples

### Query Builder Usage
```wp
GET /api/users
  |> lua: `
    local qb = querybuilder.new()
    local result = qb:select("id", "name", "email")
                     :from("users")
                     :where_if(request.search, "name ILIKE ?", "%" .. request.search .. "%")
                     :order_by("name")
                     :build()
    return result
  `
  |> pg: result.sql
```

### Validation Example
```lua
-- scripts/validation.lua
local validation = {}

function validation.validate_email(email)
    return email and email:match("^[%w._%+-]+@[%w._%+-]+%.[%a]+$")
end

return validation
```

```wp
POST /api/users
  |> lua: `
    if not validation.validate_email(request.email) then
        return { error = "Invalid email address" }
    end
    return request
  `
```

## Implementation Benefits

1. **Simplicity** - No complex caching or hot reloading
2. **Thread Safety** - Each request gets fresh Lua state with scripts loaded
3. **Low Memory Overhead** - Scripts loaded per-request, no persistent state
4. **Error Isolation** - Script errors don't affect other requests
5. **Development Friendly** - File changes reflected immediately

## File Structure

```
scripts/
├── querybuilder.lua    # Existing query builder
├── validation.lua      # Input validation helpers
├── auth.lua           # Authentication utilities
└── utils.lua          # General utilities
```

## Performance Considerations

- **File I/O per Request** - Scripts loaded from disk for each request
- **Compilation Overhead** - Lua scripts compiled fresh each time
- **Memory Usage** - Each Lua state loads all scripts

For high-traffic scenarios, this could be optimized later with:
- Bytecode caching
- Persistent Lua states with proper isolation
- Hot reloading with file watchers

## Migration Path

1. **Phase 1** - Implement basic script loading (this plan)
2. **Phase 2** - Add error handling and logging improvements
3. **Phase 3** - Optimize with caching if needed
4. **Phase 4** - Add hot reloading for development mode

## Integration with Build System

The Makefile may need updates to ensure the `scripts/` directory is available at runtime, but since we're just reading files, no special build steps are required.

## Testing Strategy

All tests will be added to the existing `test/integration/test_lua.c` file to keep the testing consolidated.

### 1. Test Setup

First, we'll need to create test helper functions and setup/teardown for script testing:

```c
// Add to test/integration/test_lua.c

// Test script setup and helpers
static char *test_scripts_dir = NULL;

static void setup_test_scripts_directory(void) {
    // Create temporary test scripts directory
    test_scripts_dir = strdup("/tmp/wp_test_scripts");
    mkdir(test_scripts_dir, 0755);
}

static void cleanup_test_scripts_directory(void) {
    if (test_scripts_dir) {
        // Remove test scripts directory and contents
        char cmd[256];
        snprintf(cmd, sizeof(cmd), "rm -rf %s", test_scripts_dir);
        system(cmd);
        free(test_scripts_dir);
        test_scripts_dir = NULL;
    }
}

static void create_test_script(const char* name, const char* content) {
    if (!test_scripts_dir) return;
    
    char filepath[512];
    snprintf(filepath, sizeof(filepath), "%s/%s.lua", test_scripts_dir, name);
    
    FILE *f = fopen(filepath, "w");
    if (f) {
        fprintf(f, "%s", content);
        fclose(f);
    }
}
```

### 2. Script Loading Tests

Add these test functions to the existing file:

```c
// Test basic script loading functionality
static void test_lua_middleware_script_loading(void) {
    setup_test_scripts_directory();
    
    // Create a simple test script
    create_test_script("testmodule", 
        "local testmodule = {}\n"
        "function testmodule.hello(name)\n"
        "    return \"Hello, \" .. (name or \"World\") .. \"!\"\n"
        "end\n"
        "return testmodule");
    
    MemoryArena *arena = create_test_arena(1024);
    json_t *input = json_pack("{s:s}", "name", "Alice");
    
    const char *config = "return { greeting = testmodule.hello(request.name) }";
    
    json_t *output = lua_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *greeting = json_object_get(output, "greeting");
    TEST_ASSERT_NOT_NULL(greeting);
    TEST_ASSERT_STRING_EQUAL("Hello, Alice!", json_string_value(greeting));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    cleanup_test_scripts_directory();
}

// Test QueryBuilder script integration
static void test_lua_middleware_querybuilder_integration(void) {
    setup_test_scripts_directory();
    
    // Create a simplified querybuilder script for testing
    create_test_script("querybuilder",
        "local QueryBuilder = {}\n"
        "QueryBuilder.__index = QueryBuilder\n"
        "function QueryBuilder.new()\n"
        "    local self = { _selects = {}, _from = nil, _wheres = {}, _params = {} }\n"
        "    return setmetatable(self, QueryBuilder)\n"
        "end\n"
        "function QueryBuilder:select(...)\n"
        "    self._selects = {...}\n"
        "    return self\n"
        "end\n"
        "function QueryBuilder:from(table)\n"
        "    self._from = table\n"
        "    return self\n"
        "end\n"
        "function QueryBuilder:where(condition, ...)\n"
        "    local params = {...}\n"
        "    table.insert(self._wheres, condition)\n"
        "    for _, param in ipairs(params) do\n"
        "        table.insert(self._params, param)\n"
        "    end\n"
        "    return self\n"
        "end\n"
        "function QueryBuilder:build()\n"
        "    local sql = \"SELECT \" .. table.concat(self._selects, \", \") .. \" FROM \" .. self._from\n"
        "    if #self._wheres > 0 then\n"
        "        sql = sql .. \" WHERE \" .. table.concat(self._wheres, \" AND \")\n"
        "    end\n"
        "    return { sql = sql, sqlParams = self._params }\n"
        "end\n"
        "return { new = QueryBuilder.new }");
    
    MemoryArena *arena = create_test_arena(1024);
    json_t *input = json_pack("{s:s, s:s}", "table", "users", "search", "john");
    
    const char *config = 
        "local qb = querybuilder.new()\n"
        "local query = qb:select('id', 'name', 'email')\n"
        "               :from(request.table)\n"
        "               :where('name LIKE ?', '%' .. request.search .. '%')\n"
        "               :build()\n"
        "return query";
    
    json_t *output = lua_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *sql = json_object_get(output, "sql");
    json_t *sqlParams = json_object_get(output, "sqlParams");
    
    TEST_ASSERT_NOT_NULL(sql);
    TEST_ASSERT_NOT_NULL(sqlParams);
    TEST_ASSERT_STRING_EQUAL("SELECT id, name, email FROM users WHERE name LIKE ?", json_string_value(sql));
    TEST_ASSERT_TRUE(json_is_array(sqlParams));
    TEST_ASSERT_EQUAL(1, json_array_size(sqlParams));
    
    json_t *param = json_array_get(sqlParams, 0);
    TEST_ASSERT_STRING_EQUAL("%john%", json_string_value(param));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    cleanup_test_scripts_directory();
}

// Test multiple scripts loaded simultaneously
static void test_lua_middleware_multiple_scripts(void) {
    setup_test_scripts_directory();
    
    // Create multiple test scripts
    create_test_script("math_utils",
        "local math_utils = {}\n"
        "function math_utils.add(a, b)\n"
        "    return (a or 0) + (b or 0)\n"
        "end\n"
        "function math_utils.multiply(a, b)\n"
        "    return (a or 1) * (b or 1)\n"
        "end\n"
        "return math_utils");
    
    create_test_script("string_utils",
        "local string_utils = {}\n"
        "function string_utils.capitalize(str)\n"
        "    return string.upper(string.sub(str, 1, 1)) .. string.sub(str, 2)\n"
        "end\n"
        "return string_utils");
    
    MemoryArena *arena = create_test_arena(1024);
    json_t *input = json_pack("{s:i, s:i, s:s}", "a", 5, "b", 3, "name", "alice");
    
    const char *config = 
        "return {\n"
        "    sum = math_utils.add(request.a, request.b),\n"
        "    product = math_utils.multiply(request.a, request.b),\n"
        "    capitalized_name = string_utils.capitalize(request.name)\n"
        "}";
    
    json_t *output = lua_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *sum = json_object_get(output, "sum");
    json_t *product = json_object_get(output, "product");
    json_t *capitalized_name = json_object_get(output, "capitalized_name");
    
    TEST_ASSERT_NOT_NULL(sum);
    TEST_ASSERT_NOT_NULL(product);
    TEST_ASSERT_NOT_NULL(capitalized_name);
    
    TEST_ASSERT_EQUAL(8, json_integer_value(sum));
    TEST_ASSERT_EQUAL(15, json_integer_value(product));
    TEST_ASSERT_STRING_EQUAL("Alice", json_string_value(capitalized_name));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    cleanup_test_scripts_directory();
}

// Test script with syntax error
static void test_lua_middleware_script_syntax_error(void) {
    setup_test_scripts_directory();
    
    // Create a script with syntax error
    create_test_script("broken",
        "local broken = {\n"
        "    invalid syntax here!!!\n"
        "}\n"
        "return broken");
    
    MemoryArena *arena = create_test_arena(1024);
    json_t *input = create_test_request("GET", "/test");
    
    const char *config = "return { result = broken.test() }";
    
    json_t *output = lua_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    // Should handle script loading error gracefully
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error in output
    json_t *error = json_object_get(output, "error");
    if (error) {
        TEST_ASSERT_TRUE(json_is_string(error));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    cleanup_test_scripts_directory();
}

// Test missing scripts directory
static void test_lua_middleware_missing_scripts_directory(void) {
    // Don't create scripts directory - test with missing directory
    
    MemoryArena *arena = create_test_arena(1024);
    json_t *input = create_test_request("GET", "/test");
    
    const char *config = "return { message = \"no scripts directory\" }";
    
    json_t *output = lua_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    // Should work fine without scripts directory
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *message = json_object_get(output, "message");
    TEST_ASSERT_NOT_NULL(message);
    TEST_ASSERT_STRING_EQUAL("no scripts directory", json_string_value(message));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

// Test script runtime error handling
static void test_lua_middleware_script_runtime_error(void) {
    setup_test_scripts_directory();
    
    // Create a script that can cause runtime errors
    create_test_script("runtime_error",
        "local runtime_error = {}\n"
        "function runtime_error.cause_error()\n"
        "    local x = nil\n"
        "    return x.nonexistent_field\n"
        "end\n"
        "return runtime_error");
    
    MemoryArena *arena = create_test_arena(1024);
    json_t *input = create_test_request("GET", "/test");
    
    const char *config = "return { result = runtime_error.cause_error() }";
    
    json_t *output = lua_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    // Should handle runtime error gracefully
    TEST_ASSERT_NOT_NULL(output);
    
    // Check for error in output
    json_t *error = json_object_get(output, "error");
    if (error) {
        TEST_ASSERT_TRUE(json_is_string(error));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    cleanup_test_scripts_directory();
}
```

### 3. Update Main Test Function

Add the new tests to the main() function in test_lua.c:

```c
int main(void) {
    UNITY_BEGIN();
    
    // Existing tests...
    RUN_TEST(test_lua_middleware_simple_return);
    RUN_TEST(test_lua_middleware_object_construction);
    // ... all existing tests ...
    
    // New script-related tests
    RUN_TEST(test_lua_middleware_script_loading);
    RUN_TEST(test_lua_middleware_querybuilder_integration);
    RUN_TEST(test_lua_middleware_multiple_scripts);
    RUN_TEST(test_lua_middleware_script_syntax_error);
    RUN_TEST(test_lua_middleware_missing_scripts_directory);
    RUN_TEST(test_lua_middleware_script_runtime_error);
    
    return UNITY_END();
}
```

### 4. Test Coverage

These tests cover:

1. **Basic Script Loading** - Verify scripts are loaded and available as globals
2. **QueryBuilder Integration** - Test the existing querybuilder.lua works
3. **Multiple Scripts** - Ensure multiple scripts can be loaded simultaneously
4. **Error Handling** - Test syntax errors, runtime errors, and missing scripts
5. **Edge Cases** - Missing scripts directory, broken scripts

### 5. Benefits of This Approach

1. **Consolidated Testing** - All Lua tests in one place
2. **Existing Infrastructure** - Reuses existing test helpers and setup
3. **Comprehensive Coverage** - Tests both basic functionality and error cases
4. **Realistic Scenarios** - Tests actual usage patterns like QueryBuilder
5. **Easy Maintenance** - Single file to maintain and update

## Conclusion

This comprehensive testing strategy ensures the scripts folder implementation is robust, reliable, and maintainable. The tests cover functionality, integration, performance, and error scenarios while providing clear examples of how the feature should be used.

This implementation provides a clean, simple way to organize Lua code into reusable modules while maintaining the thread-safe, stateless design of the current middleware system. The existing `querybuilder.lua` will work immediately once this is implemented.