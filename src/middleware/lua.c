#include <jansson.h>
#include <lua.h>
#include <lauxlib.h>
#include <lualib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>

// Arena allocation function types for middlewares
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Function prototypes
static void jansson_to_lua(lua_State *L, json_t *json);
static json_t *lua_to_jansson(lua_State *L, int index);
static json_t *lua_to_jansson_with_depth(lua_State *L, int index, int depth);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *lua_code, json_t *middleware_config, char **contentType, json_t *variables);

// Conversion functions
void jansson_to_lua(lua_State *L, json_t *json) {
  if (!json) {
    lua_pushnil(L);
    return;
  }

  switch (json_typeof(json)) {
  case JSON_NULL:
    lua_pushnil(L);
    break;
  case JSON_TRUE:
    lua_pushboolean(L, 1);
    break;
  case JSON_FALSE:
    lua_pushboolean(L, 0);
    break;
  case JSON_INTEGER:
    lua_pushinteger(L, json_integer_value(json));
    break;
  case JSON_REAL:
    lua_pushnumber(L, json_real_value(json));
    break;
  case JSON_STRING:
    lua_pushstring(L, json_string_value(json));
    break;
  case JSON_ARRAY: {
    lua_newtable(L);
    size_t size = json_array_size(json);
    for (size_t index = 0; index < size; index++) {
      json_t *value = json_array_get(json, index);
      lua_pushinteger(L, (lua_Integer)(index + 1)); // Lua arrays are 1-indexed
      jansson_to_lua(L, value);
      lua_settable(L, -3);
    }
    break;
  }
  case JSON_OBJECT: {
    lua_newtable(L);
    const char *key;
    json_t *value;
    json_object_foreach(json, key, value) {
      lua_pushstring(L, key);
      jansson_to_lua(L, value);
      lua_settable(L, -3);
    }
    break;
  }
  }
}

json_t *lua_to_jansson_with_depth(lua_State *L, int index, int depth) {
    // Prevent infinite recursion and stack overflow
    if (depth > 20) {
        return json_string("[max depth exceeded]");
    }
    
    if (lua_type(L, index) == LUA_TNIL) {
        return json_null();
    } else if (lua_type(L, index) == LUA_TBOOLEAN) {
        return json_boolean(lua_toboolean(L, index));
    } else if (lua_type(L, index) == LUA_TNUMBER) {
        if (lua_isinteger(L, index)) {
            return json_integer(lua_tointeger(L, index));
        } else {
            return json_real(lua_tonumber(L, index));
        }
    } else if (lua_type(L, index) == LUA_TSTRING) {
        return json_string(lua_tostring(L, index));
    } else if (lua_type(L, index) == LUA_TTABLE) {
        int abs_index = lua_absindex(L, index);
        
        // First pass: determine if it's an array and collect all keys
        bool is_array = true;
        int max_index = 0;
        int count = 0;
        json_t *object = json_object();
        json_t *array = NULL;
        
        // Collect all entries in a single pass
        lua_pushnil(L);
        while (lua_next(L, abs_index) != 0) {
            // Check if key is numeric and positive
            if (lua_type(L, -2) == LUA_TNUMBER && lua_isinteger(L, -2)) {
                int idx = (int)lua_tointeger(L, -2);
                if (idx > 0) {
                    if (idx > max_index) max_index = idx;
                    count++;
                } else {
                    // Non-positive integer key, treat as object
                    is_array = false;
                }
            } else {
                // Non-numeric key, treat as object
                is_array = false;
            }
            
            // Store the key-value pair for later processing
            if (lua_type(L, -2) == LUA_TSTRING) {
                const char *key = lua_tostring(L, -2);
                json_t *value = lua_to_jansson_with_depth(L, -1, depth + 1);
                if (key && value) {
                    json_object_set_new(object, key, value);
                } else if (value) {
                    json_decref(value);
                }
            }
            
            lua_pop(L, 1); // Remove value, keep key for next iteration
        }
        
        // If it's an array and we have consecutive indices, create array
        if (is_array && count > 0 && count == max_index) {
            // Create array and populate it
            array = json_array();
            for (int i = 1; i <= max_index; i++) {
                lua_pushinteger(L, i);
                lua_gettable(L, abs_index);
                json_array_append_new(array, lua_to_jansson_with_depth(L, -1, depth + 1));
                lua_pop(L, 1);
            }
            json_decref(object); // Clean up the object we created
            return array;
        } else {
            // Return the object we built during iteration
            return object;
        }
    }
    return json_null();
}

json_t *lua_to_jansson(lua_State *L, int index) {
    return lua_to_jansson_with_depth(L, index, 0);
}

// Lua panic handler to prevent crashes
static int lua_panic_handler(lua_State *L) {
    const char *msg = lua_tostring(L, -1);
    fprintf(stderr, "Lua PANIC: %s\n", msg ? msg : "unknown error");
    return 0; // Don't call abort()
}

// Middleware execute function
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *lua_code, json_t *middleware_config, char **contentType, json_t *variables) {
  (void)free_func; // Suppress unused parameter warning
  (void)contentType; // Lua middleware produces JSON output, so we don't change content type
  (void)variables; // Unused parameter
  (void)middleware_config; // Unused parameter for now
    
    // Handle null or empty lua_code
    if (!lua_code || strlen(lua_code) == 0) {
        // Return the input as-is for empty/null code
        return input ? json_incref(input) : json_null();
    }
    
    // Use the arena for any per-request allocations if needed (e.g., copying lua_code)
    size_t len = strlen(lua_code);
    char *arena_code = NULL;
    
    if (alloc_func && arena) {
        arena_code = alloc_func(arena, len + 1);
        if (!arena_code) {
            json_t *result = json_object();
            json_object_set_new(result, "error", json_string("Arena OOM for lua code"));
            return result;
        }
        memcpy(arena_code, lua_code, len);
        arena_code[len] = '\0';
    } else {
        // Fallback to regular allocation if no arena
        arena_code = malloc(len + 1);
        if (!arena_code) {
            json_t *result = json_object();
            json_object_set_new(result, "error", json_string("OOM for lua code"));
            return result;
        }
        memcpy(arena_code, lua_code, len);
        arena_code[len] = '\0';
    }
    
    bool free_arena_code = !alloc_func || !arena;  // Track if we need to free
    
    // Create a new Lua state for each execution (thread-safe)
    lua_State *L = luaL_newstate();
    if (!L) {
        fprintf(stderr, "lua: Failed to create new Lua state\n");
        if (free_arena_code) {
            free(arena_code);
        }
        return NULL;
    }
    
    // Set a panic handler to prevent crashes
    lua_atpanic(L, lua_panic_handler);
    
    luaL_openlibs(L);
    
    // Push input as 'request' global variable
    jansson_to_lua(L, input);
    lua_setglobal(L, "request");
    
    // Execute lua code
    if (luaL_dostring(L, arena_code) != LUA_OK) {
        const char *err = lua_tostring(L, -1);
        json_t *result = json_object();
        json_object_set_new(result, "error", json_string(err ? err : "Lua error"));
        lua_close(L);
        if (free_arena_code) {
            free(arena_code);
        }
        return result;
    }
    
    // Get result from stack
    json_t *output = NULL;
    if (lua_gettop(L) > 0) {
        output = lua_to_jansson(L, -1);
        lua_pop(L, 1);
    } else {
        // No return value, return the original input
        output = json_incref(input);
    }
    
    lua_close(L);
    if (free_arena_code) {
        free(arena_code);
    }
    return output;
}
