#include <microhttpd.h>
#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <sys/stat.h>
#include <stdbool.h>
#include <pthread.h>
#include "wp.h"

// Runtime state
typedef struct {
    struct MHD_Daemon *daemon;
    ASTNode *program;
    Plugin *plugins;
    int plugin_count;
    json_t *variables;
} WPRuntime;

// Global runtime instance
static WPRuntime *runtime = NULL;

// Memory arena functions
MemoryArena *arena_create(size_t size) {
    MemoryArena *arena = malloc(sizeof(MemoryArena));
    arena->memory = malloc(size);
    arena->size = size;
    arena->used = 0;
    return arena;
}

void *arena_alloc(MemoryArena *arena, size_t size) {
    if (arena->used + size > arena->size) {
        return NULL; // Out of memory
    }
    void *ptr = arena->memory + arena->used;
    arena->used += size;
    return ptr;
}

void arena_free(MemoryArena *arena) {
    free(arena->memory);
    free(arena);
}

// Thread-local storage for current arena
static pthread_key_t current_arena_key;
static pthread_once_t arena_key_once = PTHREAD_ONCE_INIT;

static void arena_key_init(void) {
    pthread_key_create(&current_arena_key, NULL);
}

// Set current arena for this thread
void set_current_arena(MemoryArena *arena) {
    pthread_once(&arena_key_once, arena_key_init);
    pthread_setspecific(current_arena_key, arena);
}

// Get current arena for this thread
MemoryArena *get_current_arena(void) {
    pthread_once(&arena_key_once, arena_key_init);
    return pthread_getspecific(current_arena_key);
}

// Custom jansson allocator functions
static void *jansson_arena_malloc(size_t size) {
    MemoryArena *arena = get_current_arena();
    if (arena) {
        return arena_alloc(arena, size);
    }
    return malloc(size); // Fallback
}

static void jansson_arena_free(void *ptr) {
    // Arena memory is freed all at once, so we don't need to do anything here
    // But we can't use free() because the pointer might be from the arena
    // When arena is NULL, this means we're in cleanup phase and should ignore
    (void)ptr; // Suppress unused parameter warning
}

// Wrapper functions for plugin interface
static void *arena_alloc_wrapper(void *arena, size_t size) {
    return arena_alloc((MemoryArena*)arena, size);
}

static void arena_free_wrapper(void *arena) {
    arena_free((MemoryArena*)arena);
}

// Plugin loading and management
int load_plugin(const char *name) {
    char plugin_path[256];
    snprintf(plugin_path, sizeof(plugin_path), "./plugins/%s.so", name);
    
    void *handle = dlopen(plugin_path, RTLD_LAZY);
    if (!handle) {
        fprintf(stderr, "Error loading plugin %s: %s\n", name, dlerror());
        return -1;
    }
    
    // Get plugin execute function
    json_t *(*execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *) = 
        (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *))dlsym(handle, "plugin_execute");
    if (!execute) {
        fprintf(stderr, "Error getting plugin_execute for %s: %s\n", name, dlerror());
        dlclose(handle);
        return -1;
    }
    
    // Add to runtime plugins
    runtime->plugins = realloc(runtime->plugins, sizeof(Plugin) * (size_t)(runtime->plugin_count + 1));
    runtime->plugins[runtime->plugin_count].name = strdup(name);
    runtime->plugins[runtime->plugin_count].handle = handle;
    runtime->plugins[runtime->plugin_count].execute = execute;
    runtime->plugin_count++;
    
    return 0;
}

Plugin *find_plugin(const char *name) {
    for (int i = 0; i < runtime->plugin_count; i++) {
        if (strcmp(runtime->plugins[i].name, name) == 0) {
            return &runtime->plugins[i];
        }
    }
    return NULL;
}

// HTTP request handling
json_t *create_request_json(struct MHD_Connection *connection, 
                           const char *url, const char *method,
                           const char *upload_data, size_t *upload_data_size) {
    (void)connection; // Suppress unused parameter warning
    json_t *request = json_object();
    
    // Set method
    json_object_set_new(request, "method", json_string(method));
    
    // Set URL
    json_object_set_new(request, "url", json_string(url));
    
    // Parse URL params (simple implementation)
    json_t *params = json_object();
    json_object_set_new(request, "params", params);
    
    // Parse query string
    json_t *query = json_object();
    json_object_set_new(request, "query", query);
    
    // Set body if present
    if (upload_data && *upload_data_size > 0) {
        json_object_set_new(request, "body", json_string(upload_data));
    } else {
        json_object_set_new(request, "body", json_null());
    }
    
    // Headers
    json_t *headers = json_object();
    json_object_set_new(request, "headers", headers);
    
    return request;
}

int execute_pipeline_with_result(PipelineStep *pipeline, json_t *request, MemoryArena *arena, 
                                json_t **final_response, int *response_code) {
    json_t *current = json_incref(request);
    *response_code = 200; // Default
    
    PipelineStep *step = pipeline;
    while (step) {
        if (strcmp(step->plugin, "result") == 0) {
            // Handle result step
            ASTNode *result_node = (ASTNode*)(uintptr_t)step->value;
            
            // Determine which condition to execute based on current state
            ResultCondition *condition = result_node->data.result_step.conditions;
            ResultCondition *selected_condition = NULL;
            
            // Check for error conditions first
            json_t *error = json_object_get(current, "error");
            if (error && json_is_string(error)) {
                // Look for error condition
                while (condition) {
                    if (strcmp(condition->condition_name, "error") == 0 ||
                        strcmp(condition->condition_name, "validationError") == 0) {
                        selected_condition = condition;
                        break;
                    }
                    condition = condition->next;
                }
            }
            
            // If no error condition found, use default "ok" condition
            if (!selected_condition) {
                condition = result_node->data.result_step.conditions;
                while (condition) {
                    if (strcmp(condition->condition_name, "ok") == 0) {
                        selected_condition = condition;
                        break;
                    }
                    condition = condition->next;
                }
            }
            
            if (selected_condition) {
                *response_code = selected_condition->status_code;
                
                // Execute the selected condition's pipeline
                if (selected_condition->pipeline) {
                    json_t *condition_result = NULL;
                    int temp_code;
                    int result = execute_pipeline_with_result(selected_condition->pipeline, 
                                                            current, arena, &condition_result, &temp_code);
                    if (result == 0 && condition_result) {
                        // json_decref(current);
                        current = condition_result;
                    }
                }
            }
            
            *final_response = current;
            return 0;
        }
        
        Plugin *plugin = find_plugin(step->plugin);
        if (!plugin) {
            fprintf(stderr, "Plugin not found: %s\n", step->plugin);
            // json_decref(current);
            return -1;
        }
        
        const char *config = step->value;
        if (step->is_variable) {
            // Look up variable value
            json_t *var_value = json_object_get(runtime->variables, step->value);
            if (var_value && json_is_string(var_value)) {
                config = json_string_value(var_value);
            }
        }
        
        json_t *result = plugin->execute(current, arena, arena_alloc_wrapper, arena_free_wrapper, config);
        if (!result) {
            fprintf(stderr, "Plugin %s failed\n", step->plugin);
            // json_decref(current);
            return -1;
        }
        
        // json_decref(current);
        current = result;
        step = step->next;
    }
    
    *final_response = current;
    return 0;
}

int execute_pipeline(PipelineStep *pipeline, json_t *request, MemoryArena *arena) {
    json_t *response = NULL;
    int response_code;
    int result = execute_pipeline_with_result(pipeline, request, arena, &response, &response_code);
    if (response) {
        // json_decref(response);
    }
    return result;
}

bool match_route(const char *pattern, const char *url, json_t *params) {
    // Skip leading slash if present
    if (url[0] == '/') {
        url++;
    }
    
    // Split pattern and URL into parts using strtok_r for thread safety
    char *pattern_copy = strdup(pattern);
    char *url_copy = strdup(url);
    
    char *pattern_parts[64];  // Max 64 parts
    char *url_parts[64];
    int pattern_count = 0;
    int url_count = 0;
    
    char *saveptr1 = NULL;
    char *saveptr2 = NULL;
    char *pattern_part = strtok_r(pattern_copy, "/", &saveptr1);
    while (pattern_part && pattern_count < 64) {
        pattern_parts[pattern_count++] = pattern_part;
        pattern_part = strtok_r(NULL, "/", &saveptr1);
    }
    
    char *url_part = strtok_r(url_copy, "/", &saveptr2);
    while (url_part && url_count < 64) {
        url_parts[url_count++] = url_part;
        url_part = strtok_r(NULL, "/", &saveptr2);
    }
    
    // If different number of parts, no match
    if (pattern_count != url_count) {
        free(pattern_copy);
        free(url_copy);
        return false;
    }
    
    // Compare parts
    for (int i = 0; i < pattern_count; i++) {
        if (pattern_parts[i][0] == ':') {
            // Parameter - extract the parameter name (remove the colon)
            char *param_name = pattern_parts[i] + 1;
            // Try to parse as integer, otherwise store as string
            char *endptr = NULL;
            long val = strtol(url_parts[i], &endptr, 10);
            if (*endptr == '\0') {
                json_object_set_new(params, param_name, json_integer(val));
            } else {
                json_object_set_new(params, param_name, json_string(url_parts[i]));
            }
        } else if (strcmp(pattern_parts[i], url_parts[i]) != 0) {
            free(pattern_copy);
            free(url_copy);
            return false;
        }
    }
    
    free(pattern_copy);
    free(url_copy);
    return true;
}

static enum MHD_Result handle_request(void *cls, struct MHD_Connection *connection,
                         const char *url, const char *method,
                         const char *version, const char *upload_data,
                         size_t *upload_data_size, void **con_cls) {
    
    (void)cls; // Suppress unused parameter warning
    (void)version; // Suppress unused parameter warning
    
    if (*con_cls == NULL) {
        *con_cls = (void *)1;
        return MHD_YES;
    }
    
    MemoryArena *arena = arena_create(1024 * 1024); // 1MB arena
    set_current_arena(arena); // Set arena for this thread
    
    json_t *request = create_request_json(connection, url, method, 
                                         upload_data, upload_data_size);
    
    // Find matching route
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_ROUTE_DEFINITION) {
            if (strcmp(stmt->data.route_def.method, method) == 0) {
                json_t *params = json_object_get(request, "params");
                if (match_route(stmt->data.route_def.route, url, params)) {
                    // If pipeline is empty, return the request object as the response
                    if (!stmt->data.route_def.pipeline) {
                        set_current_arena(NULL);
                        char *response_str = json_dumps(request, JSON_COMPACT);
                        set_current_arena(arena);
                        struct MHD_Response *mhd_response = 
                            MHD_create_response_from_buffer(strlen(response_str),
                                                           (void*)response_str,
                                                           MHD_RESPMEM_MUST_FREE);
                        MHD_add_response_header(mhd_response, "Content-Type", "application/json");
                        (void)MHD_queue_response(connection, 200, mhd_response);
                        MHD_destroy_response(mhd_response);
                        set_current_arena(NULL);
                        arena_free(arena);
                        return MHD_YES;
                    }
                    // Execute pipeline with result handling
                    json_t *final_response = NULL;
                    int response_code = 200;
                    
                    int result = execute_pipeline_with_result(stmt->data.route_def.pipeline, 
                                                            request, arena, &final_response, &response_code);
                    
                    if (result == 0 && final_response) {
                        // Convert JSON response to string - use regular malloc to avoid arena issues
                        set_current_arena(NULL); // Temporarily disable arena for json_dumps
                        char *response_str = json_dumps(final_response, JSON_COMPACT);
                        set_current_arena(arena); // Re-enable arena
                        
                        struct MHD_Response *mhd_response = 
                            MHD_create_response_from_buffer(strlen(response_str),
                                                           (void*)response_str,
                                                           MHD_RESPMEM_MUST_FREE);
                        
                        // Add JSON content type header
                        MHD_add_response_header(mhd_response, "Content-Type", "application/json");
                        
                        (void)MHD_queue_response(connection, (unsigned int)response_code, mhd_response);
                        MHD_destroy_response(mhd_response);
                        
                        // json_decref(final_response);
                    } else {
                        // Error in pipeline execution
                        const char *error_response = "{\"error\": \"Internal server error\"}";
                        struct MHD_Response *mhd_response = 
                            MHD_create_response_from_buffer(strlen(error_response),
                                                           (void*)(uintptr_t)error_response,
                                                           MHD_RESPMEM_PERSISTENT);
                        (void)MHD_queue_response(connection, MHD_HTTP_INTERNAL_SERVER_ERROR, mhd_response);
                        MHD_destroy_response(mhd_response);
                    }
                    
                    // Clear current arena before freeing to prevent jansson from accessing freed memory
                    set_current_arena(NULL);
                    arena_free(arena);
                    return MHD_YES;
                }
            }
        }
    }
    
    // No route found
    const char *response = "{\"error\": \"Not found\"}";
    struct MHD_Response *mhd_response = 
        MHD_create_response_from_buffer(strlen(response),
                                       (void*)(uintptr_t)response,
                                       MHD_RESPMEM_PERSISTENT);
    (void)MHD_queue_response(connection, MHD_HTTP_NOT_FOUND, mhd_response);
    MHD_destroy_response(mhd_response);
    
    // Clear current arena before freeing to prevent jansson from accessing freed memory
    set_current_arena(NULL);
    arena_free(arena);
    return MHD_YES;
}

// Runtime initialization
int wp_runtime_init(const char *wp_file) {
    printf("Initializing runtime\n");
    
    // Check if we can access microhttpd functions
    printf("Checking microhttpd availability...\n");
    
    runtime = malloc(sizeof(WPRuntime));
    runtime->plugins = NULL;
    runtime->plugin_count = 0;
    runtime->variables = json_object();
    
    // Set up jansson to use arena allocators
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    // Parse wp file
    FILE *file = fopen(wp_file, "r");
    if (!file) {
        fprintf(stderr, "Error: Could not open file '%s'\n", wp_file);
        return -1;
    }
    
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    char *source = malloc((size_t)file_size + 1);
    fread(source, 1, (size_t)file_size, file);
    source[file_size] = '\0';
    fclose(file);

    printf("Tokenizing and parsing\n");
    
    // Tokenize and parse
    int token_count;
    Token *tokens = lexer_tokenize(source, &token_count);
    Parser *parser = parser_new(tokens, token_count);
    runtime->program = parser_parse(parser);

    printf("Parsed program\n");
    
    // Process variable assignments
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_VARIABLE_ASSIGNMENT) {
            json_object_set_new(runtime->variables, stmt->data.var_assign.name,
                               json_string(stmt->data.var_assign.value));
        }
    }
    
    // Load required plugins
    printf("Loading plugins...\n");
    if (load_plugin("jq") != 0) {
        printf("Warning: Failed to load jq plugin\n");
    } else {
        printf("Loaded jq plugin successfully\n");
    }
    
    if (load_plugin("lua") != 0) {
        printf("Warning: Failed to load lua plugin\n");
    } else {
        printf("Loaded lua plugin successfully\n");
    }
    
    if (load_plugin("pg") != 0) {
        printf("Warning: Failed to load pg plugin\n");
    } else {
        printf("Loaded pg plugin successfully\n");
    }
    
    // Start HTTP server
    printf("Starting HTTP server on port 8080...\n");
    
    // Try to start the daemon with more detailed error handling
    runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                      8080, NULL, NULL,
                                      &handle_request, NULL,
                                      MHD_OPTION_END);
    
    if (!runtime->daemon) {
        fprintf(stderr, "Error starting HTTP server on port 8080\n");
        
        // Try alternative port
        printf("Trying port 8081...\n");
        runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                          8081, NULL, NULL,
                                          &handle_request, NULL,
                                          MHD_OPTION_END);
        
        if (!runtime->daemon) {
            fprintf(stderr, "Error starting HTTP server on port 8081 as well\n");
            fprintf(stderr, "Check if ports are in use or if you have permission to bind to them\n");
            return -1;
        } else {
            printf("HTTP server started successfully on port 8081\n");
        }
    } else {
        printf("HTTP server started successfully on port 8080\n");
    }
    
    parser_free(parser);
    free_tokens(tokens, token_count);
    free(source);
    
    return 0;
}

void wp_runtime_cleanup() {
    if (runtime) {
        if (runtime->daemon) {
            MHD_stop_daemon(runtime->daemon);
        }
        
        // Cleanup plugins
        for (int i = 0; i < runtime->plugin_count; i++) {
            dlclose(runtime->plugins[i].handle);
            free(runtime->plugins[i].name);
        }
        free(runtime->plugins);
        
        // json_decref(runtime->variables);
        free_ast(runtime->program);
        free(runtime);
    }
} 
