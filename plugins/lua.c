#include <jansson.h>
#include <lua.h>
#include <lauxlib.h>
#include <lualib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Arena allocation function types for plugins
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Conversion functions
void jansson_to_lua(lua_State *L, json_t *json) {
    if (json_is_null(json)) {
        lua_pushnil(L);
    } else if (json_is_true(json)) {
        lua_pushboolean(L, 1);
    } else if (json_is_false(json)) {
        lua_pushboolean(L, 0);
    } else if (json_is_number(json)) {
        lua_pushnumber(L, json_number_value(json));
    } else if (json_is_string(json)) {
        lua_pushstring(L, json_string_value(json));
    } else if (json_is_array(json)) {
        lua_newtable(L);
        size_t index;
        json_t *value;
        json_array_foreach(json, index, value) {
            lua_pushnumber(L, index + 1); // Lua arrays are 1-indexed
            jansson_to_lua(L, value);
            lua_settable(L, -3);
        }
    } else if (json_is_object(json)) {
        lua_newtable(L);
        const char *key;
        json_t *value;
        json_object_foreach(json, key, value) {
            lua_pushstring(L, key);
            jansson_to_lua(L, value);
            lua_settable(L, -3);
        }
    } else {
        lua_pushnil(L);
    }
}

json_t *lua_to_jansson(lua_State *L, int index) {
    int type = lua_type(L, index);
    
    switch (type) {
        case LUA_TNIL:
            return json_null();
        case LUA_TBOOLEAN:
            return json_boolean(lua_toboolean(L, index));
        case LUA_TNUMBER:
            return json_real(lua_tonumber(L, index));
        case LUA_TSTRING:
            return json_string(lua_tostring(L, index));
        case LUA_TTABLE: {
            // Check if it's an array or object
            bool is_array = true;
            int max_index = 0;
            int count = 0;
            int abs_index = lua_absindex(L, index);
            
            lua_pushnil(L);
            while (lua_next(L, abs_index) != 0) {
                if (lua_type(L, -2) == LUA_TNUMBER) {
                    int idx = lua_tonumber(L, -2);
                    if (idx > max_index) max_index = idx;
                    count++;
                } else {
                    is_array = false;
                    lua_pop(L, 1);
                    break;
                }
                lua_pop(L, 1);
            }
            
            if (is_array && count == max_index) {
                // It's an array
                json_t *array = json_array();
                for (int i = 1; i <= max_index; i++) {
                    lua_pushnumber(L, i);
                    lua_gettable(L, abs_index);
                    json_array_append_new(array, lua_to_jansson(L, -1));
                    lua_pop(L, 1);
                }
                return array;
            } else {
                // It's an object
                json_t *object = json_object();
                lua_pushnil(L);
                while (lua_next(L, abs_index) != 0) {
                    const char *key = lua_tostring(L, -2);
                    if (key) {
                        json_object_set_new(object, key, lua_to_jansson(L, -1));
                    }
                    lua_pop(L, 1);
                }
                return object;
            }
        }
        default:
            return json_null();
    }
}

// Plugin execute function
json_t *plugin_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *lua_code) {
    // Use the arena for any per-request allocations if needed (e.g., copying lua_code)
    size_t len = strlen(lua_code);
    char *arena_code = alloc_func(arena, len + 1);
    if (!arena_code) {
        json_t *result = json_object();
        json_object_set_new(result, "error", json_string("Arena OOM for lua code"));
        return result;
    }
    memcpy(arena_code, lua_code, len);
    arena_code[len] = '\0';
    // Create a new Lua state for each execution (thread-safe)
    lua_State *L = luaL_newstate();
    if (!L) {
        fprintf(stderr, "lua: Failed to create new Lua state\n");
        return NULL;
    }
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
    return output;
}