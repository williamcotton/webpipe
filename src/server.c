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
    Middleware *middleware;
    int middleware_count;
    json_t *variables;
    ParseContext *parse_ctx;
} WPRuntime;

// Global runtime instance
static WPRuntime *runtime = NULL;

// Memory arena functions
MemoryArena *arena_create(size_t size) {
    if (size == 0) {
        return NULL;
    }
    
    MemoryArena *arena = malloc(sizeof(MemoryArena));
    if (!arena) {
        return NULL;
    }
    
    arena->memory = malloc(size);
    if (!arena->memory) {
        free(arena);
        return NULL;
    }
    
    arena->size = size;
    arena->used = 0;
    return arena;
}

void *arena_alloc(MemoryArena *arena, size_t size) {
    if (!arena || size == 0) {
        return NULL;
    }
    
    // Check if arena was already freed (defensive programming)
    if (!arena->memory) {
        return NULL;
    }
    
    // Align to 8-byte boundary for struct alignment
    size_t alignment = 8;
    size_t aligned_used = (arena->used + alignment - 1) & ~(alignment - 1);
    
    if (aligned_used + size > arena->size) {
        return NULL; // Out of memory
    }
    
    void *ptr = arena->memory + aligned_used;
    arena->used = aligned_used + size;
    return ptr;
}

void arena_free(MemoryArena *arena) {
    if (!arena) {
        return;
    }
    if (arena->memory) {
        free(arena->memory);
        arena->memory = NULL; // Mark as freed
    }
    free(arena);
}

// Arena string allocation functions
char *arena_strdup(MemoryArena *arena, const char *str) {
    if (!str) return NULL;
    
    size_t len = strlen(str);
    char *copy = arena_alloc(arena, len + 1);
    if (copy) {
        memcpy(copy, str, len);
        copy[len] = '\0';
    }
    return copy;
}

char *arena_strndup(MemoryArena *arena, const char *str, size_t n) {
    if (!str) return NULL;
    
    size_t len = strlen(str);
    if (len > n) len = n;
    
    char *copy = arena_alloc(arena, len + 1);
    if (copy) {
        strncpy(copy, str, len);
        copy[len] = '\0';
    }
    return copy;
}

// Parse context management
ParseContext *parse_context_create(void) {
    ParseContext *ctx = malloc(sizeof(ParseContext));
    if (!ctx) return NULL;
    
    ctx->parse_arena = arena_create(1024 * 1024);    // 1MB for parsing
    ctx->runtime_arena = arena_create(256 * 1024);   // 256KB for runtime data
    
    if (!ctx->parse_arena || !ctx->runtime_arena) {
        if (ctx->parse_arena) arena_free(ctx->parse_arena);
        if (ctx->runtime_arena) arena_free(ctx->runtime_arena);
        free(ctx);
        return NULL;
    }
    
    return ctx;
}

void parse_context_destroy(ParseContext *ctx) {
    if (!ctx) return;
    
    if (ctx->parse_arena) arena_free(ctx->parse_arena);
    if (ctx->runtime_arena) arena_free(ctx->runtime_arena);
    free(ctx);
}

// Thread-local storage for current arena
static _Thread_local MemoryArena *currentArena = NULL;

// Set current arena for this thread
void set_current_arena(MemoryArena *arena) {
    currentArena = arena;
}

// Get current arena for this thread
MemoryArena *get_current_arena(void) {
    return currentArena;
}

// Custom jansson allocator functions
void *jansson_arena_malloc(size_t size) {
    MemoryArena *arena = get_current_arena();
    if (arena && arena->memory) {  // Check if arena is valid
        void *ptr = arena_alloc(arena, size);
        if (ptr) {
            return ptr;
        }
        // Arena is full, fallback to malloc
        fprintf(stderr, "WARNING: Arena full, falling back to malloc for size=%zu\n", size);
    }
    // Fallback to malloc if no arena or arena is full/invalid
    return malloc(size);
}

void jansson_arena_free(void *ptr) {
    // Arena memory is freed all at once, so we don't need to do anything here
    (void)ptr;
}

// Wrapper functions for middleware interface
static void *arena_alloc_wrapper(void *arena, size_t size) {
    return arena_alloc((MemoryArena*)arena, size);
}

static void arena_free_wrapper(void *arena) {
    arena_free((MemoryArena*)arena);
}

// Middleware loading and management
int load_middleware(const char *name) {
    // Check if runtime is initialized
    if (!runtime) {
        fprintf(stderr, "Error: Runtime not initialized\n");
        return -1;
    }
    
    char middleware_path[256];
    snprintf(middleware_path, sizeof(middleware_path), "./middleware/%s.so", name);
    
    void *handle = dlopen(middleware_path, RTLD_LAZY);
    if (!handle) {
        fprintf(stderr, "Error loading middleware %s: %s\n", name, dlerror());
        return -1;
    }
    
    // Get middleware execute function
    json_t *(*execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *) = 
        (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *))dlsym(handle, "middleware_execute");
    if (!execute) {
        fprintf(stderr, "Error getting middleware_execute for %s: %s\n", name, dlerror());
        dlclose(handle);
        return -1;
    }
    
    // Add to runtime middleware
    runtime->middleware = realloc(runtime->middleware, sizeof(Middleware) * (size_t)(runtime->middleware_count + 1));
    runtime->middleware[runtime->middleware_count].name = strdup(name);
    runtime->middleware[runtime->middleware_count].handle = handle;
    runtime->middleware[runtime->middleware_count].execute = execute;
    runtime->middleware_count++;
    
    return 0;
}

Middleware *find_middleware(const char *name) {
    // Check if runtime is initialized
    if (!runtime) {
        return NULL;
    }
    
    for (int i = 0; i < runtime->middleware_count; i++) {
        if (strcmp(runtime->middleware[i].name, name) == 0) {
            return &runtime->middleware[i];
        }
    }
    return NULL;
}

// Request completion callback for arena cleanup
static void request_completed(void *cls, struct MHD_Connection *connection,
                              void **con_cls, enum MHD_RequestTerminationCode toe) {
    (void)cls;
    (void)connection;
    (void)toe;
    
    if (*con_cls != NULL) {
        // Check if it's a PostData structure by checking the magic number
        PostData *post_data = (PostData *)*con_cls;
        if (post_data->magic == POST_DATA_MAGIC) {
            // It's a PostData structure, clean up post processor if exists
            if (post_data->post_processor) {
                MHD_destroy_post_processor(post_data->post_processor);
            }
            // Free the arena
            MemoryArena *arena = post_data->arena;
            set_current_arena(NULL);
            arena_free(arena);
        } else {
            // It's just an arena pointer (for non-POST requests)
            MemoryArena *arena = (MemoryArena *)*con_cls;
            set_current_arena(NULL);
            arena_free(arena);
        }
        *con_cls = NULL;
    }
}

// Form data iterator for MHD_PostProcessor
static enum MHD_Result form_data_iterator(void *cls, enum MHD_ValueKind kind, 
                                         const char *key, const char *filename, 
                                         const char *content_type, const char *transfer_encoding,
                                         const char *data, uint64_t off, size_t size) {
    (void)kind;
    (void)filename;
    (void)content_type;
    (void)transfer_encoding;
    (void)off;
    
    PostData *post_data = (PostData *)cls;
    
    if (!post_data || !post_data->form_data || !key) {
        return MHD_NO;
    }
    
    if (size > 0 && data) {
        // Create a null-terminated string from the data
        char *value = arena_alloc(post_data->arena, size + 1);
        if (!value) {
            return MHD_NO;
        }
        memcpy(value, data, size);
        value[size] = '\0';
        
        // Add to form data JSON object
        json_object_set_new(post_data->form_data, key, json_string(value));
    }
    
    return MHD_YES;
}

// HTTP request handling
json_t *create_request_json(struct MHD_Connection *connection, 
                           const char *url, const char *method,
                           PostData *post_data) {
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
    if (post_data) {
        if (post_data->is_form_data && post_data->form_data) {
            // Use parsed form data
            json_object_set_new(request, "body", json_deep_copy(post_data->form_data));
        } else if (post_data->post_data && post_data->post_data_size > 0) {
            // Try to parse as JSON first
            json_error_t error;
            json_t *json_body = json_loadb(post_data->post_data, post_data->post_data_size, 0, &error);
            if (json_body) {
                json_object_set_new(request, "body", json_body);
            } else {
                // If not valid JSON, store as string
                json_object_set_new(request, "body", json_stringn(post_data->post_data, post_data->post_data_size));
            }
        } else {
            json_object_set_new(request, "body", json_null());
        }
    } else {
        json_object_set_new(request, "body", json_null());
    }
    
    // Headers
    json_t *headers = json_object();
    json_object_set_new(request, "headers", headers);
    
    return request;
}

// Helper function to check if JSON has errors array
static bool has_errors(json_t *json) {
    json_t *errors = json_object_get(json, "errors");
    return errors != NULL && json_is_array(errors) && json_array_size(errors) > 0;
}

// Helper function to get first error type
static const char *get_first_error_type(json_t *json) {
    json_t *errors = json_object_get(json, "errors");
    if (!errors || !json_is_array(errors)) return NULL;
    
    json_t *first_error = json_array_get(errors, 0);
    if (!first_error) return NULL;
    
    json_t *type_json = json_object_get(first_error, "type");
    if (!type_json || !json_is_string(type_json)) return NULL;
    
    return json_string_value(type_json);
}

// Helper function to send response with flexible content type
static enum MHD_Result send_response(struct MHD_Connection *connection, 
                                   json_t *json_data, int status_code, const char *content_type, MemoryArena *arena) {
    char *response_str = NULL;
    size_t response_len = 0;
    
    // Validate inputs
    if (!connection) {
        fprintf(stderr, "Error: connection is NULL in send_response\n");
        return MHD_NO;
    }
    
    if (!json_data) {
        fprintf(stderr, "Error: json_data is NULL in send_response\n");
        // Create a fallback error response
        response_str = arena_strdup(arena, "{\"error\":\"Internal server error\"}");
        response_len = strlen(response_str);
        content_type = "application/json";
        status_code = 500;
    } else {
        // Default content type if not specified
        if (!content_type) {
            content_type = "application/json";
        }
        
        // Handle different content types
        if (strcmp(content_type, "application/json") == 0) {
            // JSON response
            response_str = json_dumps(json_data, JSON_COMPACT);
            if (!response_str) {
                fprintf(stderr, "Error: json_dumps failed in send_response\n");
                response_str = arena_strdup(arena, "{\"error\":\"JSON serialization failed\"}");
                response_len = strlen(response_str);
            } else {
                response_len = strlen(response_str);
            }
        } else if (strcmp(content_type, "text/html") == 0 || 
                   strcmp(content_type, "text/plain") == 0 ||
                   strncmp(content_type, "text/", 5) == 0) {
            // HTML or text response - extract string from JSON
            if (json_is_string(json_data)) {
                const char *content = json_string_value(json_data);
                if (content) {
                    response_len = strlen(content);
                    // Use arena_strdup to prevent memory leak
                    response_str = arena_strdup(arena, content);
                } else {
                    fprintf(stderr, "Error: json_string_value returned NULL\n");
                    response_str = arena_strdup(arena, "Internal server error");
                    response_len = strlen(response_str);
                    content_type = "text/plain";
                }
            } else {
                // Fallback to JSON if not a string
                response_str = json_dumps(json_data, JSON_COMPACT);
                if (!response_str) {
                    fprintf(stderr, "Error: json_dumps fallback failed\n");
                    response_str = arena_strdup(arena, "{\"error\":\"JSON serialization failed\"}");
                }
                response_len = strlen(response_str);
                content_type = "application/json";
            }
        } else {
            // Default to JSON for unknown content types
            response_str = json_dumps(json_data, JSON_COMPACT);
            if (!response_str) {
                fprintf(stderr, "Error: json_dumps failed for unknown content type\n");
                response_str = arena_strdup(arena, "{\"error\":\"JSON serialization failed\"}");
            }
            response_len = strlen(response_str);
            content_type = "application/json";
        }
    }
    
    // Final check that we have a valid response string
    if (!response_str) {
        fprintf(stderr, "Error: response_str is still NULL after processing\n");
        response_str = "{\"error\":\"Critical error\"}";
        response_len = strlen(response_str);
        content_type = "application/json";
    }
    
    struct MHD_Response *mhd_response =
        MHD_create_response_from_buffer(
            response_len, (void *)response_str,
            MHD_RESPMEM_PERSISTENT);
    
    MHD_add_response_header(mhd_response, "Content-Type", content_type);
    (void)MHD_queue_response(connection, (unsigned int)status_code, mhd_response);
    MHD_destroy_response(mhd_response);
    
    return MHD_YES;
}

// Helper function to send JSON response (backward compatibility)
static enum MHD_Result send_json_response(struct MHD_Connection *connection, 
                                         json_t *json_data, int status_code, MemoryArena *arena) {
    return send_response(connection, json_data, status_code, "application/json", arena);
}

// Helper function to send error response
static enum MHD_Result send_error_response(struct MHD_Connection *connection, 
                                          const char *error_msg, int status_code) {
    struct MHD_Response *mhd_response = 
        MHD_create_response_from_buffer(strlen(error_msg),
                                       (void*)(uintptr_t)error_msg,
                                       MHD_RESPMEM_PERSISTENT);
    (void)MHD_queue_response(connection, (unsigned int)status_code, mhd_response);
    MHD_destroy_response(mhd_response);
    return MHD_YES;
}

// Helper function to process a matched route
static enum MHD_Result process_route(struct MHD_Connection *connection,
                                    ASTNode *route_stmt, json_t *request, 
                                    MemoryArena *arena) {
    // If pipeline is empty, return the request object as the response
    if (!route_stmt->data.route_def.pipeline) {
        set_current_arena(arena);
        return send_json_response(connection, request, 200, arena);
    }
    
    // Execute pipeline with result handling
    json_t *final_response = NULL;
    int response_code = 200;
    char *content_type = NULL;
    
    int result = execute_pipeline_with_result(route_stmt->data.route_def.pipeline, 
                                            request, arena, &final_response, &response_code, &content_type);
    
    if (result == 0 && final_response) {
        set_current_arena(arena);
        return send_response(connection, final_response, response_code, content_type, arena);
    } else {
        // Error in pipeline execution
        return send_error_response(connection, 
                                 "{\"error\": \"Internal server error\"}", 
                                 MHD_HTTP_INTERNAL_SERVER_ERROR);
    }
}

// Helper function to find and process matching route
static enum MHD_Result find_and_process_route(struct MHD_Connection *connection,
                                             const char *url, const char *method,
                                             json_t *request, MemoryArena *arena) {
    // Find matching route
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_ROUTE_DEFINITION) {
            if (strcmp(stmt->data.route_def.method, method) == 0) {
                json_t *params = json_object_get(request, "params");
                if (match_route(stmt->data.route_def.route, url, params)) {
                    return process_route(connection, stmt, request, arena);
                }
            }
        }
    }
    
    // No route found
    return send_error_response(connection, 
                             "{\"error\": \"Not found\"}", 
                             MHD_HTTP_NOT_FOUND);
}

int execute_pipeline_with_result(PipelineStep *pipeline, json_t *request, MemoryArena *arena, 
                                json_t **final_response, int *response_code, char **content_type) {
    json_t *current = request;
    *response_code = 200; // Default
    *content_type = arena_strdup(arena, "application/json"); // Default content type
    
    PipelineStep *step = pipeline;
    while (step) {
        if (strcmp(step->middleware, "result") == 0) {
            // Handle result step
            ASTNode *result_node = (ASTNode*)(uintptr_t)step->value;
            
            // Determine which condition to execute based on current state
            ResultCondition *condition = result_node->data.result_step.conditions;
            ResultCondition *selected_condition = NULL;
            
            // Check for error conditions first using new standardized format
            if (has_errors(current)) {
                const char *error_type = get_first_error_type(current);
                if (error_type) {
                    // Look for matching error condition
                    while (condition) {
                        if (strcmp(condition->condition_name, error_type) == 0) {
                            selected_condition = condition;
                            break;
                        }
                        condition = condition->next;
                    }
                    
                    // If no specific error condition found, look for "default" condition
                    if (!selected_condition) {
                        condition = result_node->data.result_step.conditions;
                        while (condition) {
                            if (strcmp(condition->condition_name, "default") == 0) {
                                selected_condition = condition;
                                break;
                            }
                            condition = condition->next;
                        }
                    }
                }
            }
            
            // If no error condition found, use "ok" condition
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
                                                            current, arena, &condition_result, &temp_code, content_type);
                    // Ensure arena context is still set after recursive execution
                    set_current_arena(arena);
                    if (result == 0 && condition_result) {
                        current = condition_result;
                    }
                }
            }
            
            *final_response = current;
            return 0;
        }
        
        Middleware *middleware = find_middleware(step->middleware);
        if (!middleware) {
            fprintf(stderr, "Middleware not found: %s\n", step->middleware);
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
        
        // Ensure arena context is set before middleware execution
        set_current_arena(arena);
        json_t *result = middleware->execute(current, arena, arena_alloc_wrapper, arena_free_wrapper, config, content_type, runtime->variables);
        // Ensure arena context is still set after middleware execution
        set_current_arena(arena);
        if (!result) {
            fprintf(stderr, "Middleware %s failed\n", step->middleware);
            return -1;
        }
        
        // Check for errors after each step and jump to result block if found
        if (has_errors(result)) {
            // Find the result step in the remaining pipeline
            PipelineStep *remaining_step = step->next;
            while (remaining_step) {
                if (strcmp(remaining_step->middleware, "result") == 0) {
                    // Execute result step with error
                    ASTNode *result_node = (ASTNode*)(uintptr_t)remaining_step->value;
                    
                    ResultCondition *condition = result_node->data.result_step.conditions;
                    ResultCondition *selected_condition = NULL;
                    
                    const char *error_type = get_first_error_type(result);
                    if (error_type) {
                        // Look for matching error condition
                        while (condition) {
                            if (strcmp(condition->condition_name, error_type) == 0) {
                                selected_condition = condition;
                                break;
                            }
                            condition = condition->next;
                        }
                        
                        // If no specific error condition found, look for "default" condition
                        if (!selected_condition) {
                            condition = result_node->data.result_step.conditions;
                            while (condition) {
                                if (strcmp(condition->condition_name, "default") == 0) {
                                    selected_condition = condition;
                                    break;
                                }
                                condition = condition->next;
                            }
                        }
                    }
                    
                    if (selected_condition) {
                        *response_code = selected_condition->status_code;
                        
                        // Execute the selected condition's pipeline
                        if (selected_condition->pipeline) {
                            json_t *condition_result = NULL;
                            int temp_code;
                            int exec_result = execute_pipeline_with_result(selected_condition->pipeline, 
                                                                    result, arena, &condition_result, &temp_code, content_type);
                            // Ensure arena context is still set after recursive execution
                            set_current_arena(arena);
                            if (exec_result == 0 && condition_result) {
                                current = condition_result;
                            }
                        } else {
                            current = result;
                        }
                    }
                    
                    *final_response = current;
                    return 0;
                }
                remaining_step = remaining_step->next;
            }
        }
        
        current = result;
        step = step->next;
    }
    
    *final_response = current;
    return 0;
}

int execute_pipeline(PipelineStep *pipeline, json_t *request, MemoryArena *arena) {
    json_t *response = NULL;
    int response_code;
    char *content_type = NULL;
    int result = execute_pipeline_with_result(pipeline, request, arena, &response, &response_code, &content_type);
    return result;
}

bool match_route(const char *pattern, const char *url, json_t *params) {
    // Check for null params
    if (!params) {
        return false;
    }
    
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
            // Always store URL parameters as strings
            json_t *str_val = json_string(url_parts[i]);
            if (str_val) {
                json_object_set_new(params, param_name, str_val);
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
        MemoryArena *arena = arena_create(1024 * 1024 * 5); // 5MB arena
        if (!arena) {
            return MHD_NO;
        }
        set_current_arena(arena); // Set arena for this thread IMMEDIATELY
        
        // For POST, PUT, and PATCH requests, we need to collect the data
        if (strcmp(method, "POST") == 0 || strcmp(method, "PUT") == 0 || strcmp(method, "PATCH") == 0) {
            PostData *post_data = arena_alloc(arena, sizeof(PostData));
            if (!post_data) {
                arena_free(arena);
                return MHD_NO;
            }
            post_data->magic = POST_DATA_MAGIC;
            post_data->arena = arena;
            post_data->post_data = NULL;
            post_data->post_data_size = 0;
            post_data->post_data_capacity = 0;
            post_data->post_processor = NULL;
            post_data->form_data = NULL;
            post_data->is_form_data = 0;
            
            // Check Content-Type to determine if this is form data
            const char *content_type = MHD_lookup_connection_value(connection, MHD_HEADER_KIND, "Content-Type");
            if (content_type && strstr(content_type, "application/x-www-form-urlencoded")) {
                // This is form data, create post processor
                post_data->post_processor = MHD_create_post_processor(connection, 1024, form_data_iterator, post_data);
                if (post_data->post_processor) {
                    post_data->is_form_data = 1;
                    post_data->form_data = json_object();
                }
            }
            
            *con_cls = post_data;
        } else {
            *con_cls = arena;
        }
        return MHD_YES;
    }
    
    // Handle POST, PUT, and PATCH data collection
    if (strcmp(method, "POST") == 0 || strcmp(method, "PUT") == 0 || strcmp(method, "PATCH") == 0) {
        PostData *post_data = (PostData *)*con_cls;
        if (!post_data || !post_data->arena) {
            return MHD_NO;
        }
        set_current_arena(post_data->arena);
        
        // If we have upload data, process it
        if (*upload_data_size > 0) {
            if (post_data->is_form_data && post_data->post_processor) {
                // Process form data using libmicrohttpd's post processor
                enum MHD_Result result = MHD_post_process(post_data->post_processor, upload_data, *upload_data_size);
                *upload_data_size = 0; // Mark as consumed
                return result;
            } else {
                // Handle as raw data (JSON or other)
                // Ensure we have enough capacity
                size_t new_size = post_data->post_data_size + *upload_data_size;
                if (new_size > post_data->post_data_capacity) {
                    size_t new_capacity = new_size + 1024; // Add some buffer
                    char *new_buffer = arena_alloc(post_data->arena, new_capacity);
                    if (!new_buffer) {
                        return MHD_NO;
                    }
                    if (post_data->post_data) {
                        memcpy(new_buffer, post_data->post_data, post_data->post_data_size);
                    }
                    post_data->post_data = new_buffer;
                    post_data->post_data_capacity = new_capacity;
                }
                
                // Append new data
                memcpy(post_data->post_data + post_data->post_data_size, upload_data, *upload_data_size);
                post_data->post_data_size += *upload_data_size;
                *upload_data_size = 0; // Mark as consumed
                
                return MHD_YES; // Continue receiving data
            }
        }
        
        // No more data to receive, process the request
        json_t *request = create_request_json(connection, url, method, post_data);
        
        // Continue with normal request processing...
        MemoryArena *arena = post_data->arena;
        set_current_arena(arena);
        
        // Process the request using the extracted helper function
        return find_and_process_route(connection, url, method, request, arena);
    }
    
    // Handle non-POST requests
    MemoryArena *arena = (MemoryArena *)*con_cls;
    if (!arena) {
        return MHD_NO;
    }
    set_current_arena(arena); // ALWAYS set arena for this thread on each request
    
    json_t *request = create_request_json(connection, url, method, NULL);
    
    // Process the request using the extracted helper function
    return find_and_process_route(connection, url, method, request, arena);
}

// Function to collect unique middleware names from AST
void collect_middleware_names_from_ast(ASTNode *node, char **middleware_names, int *middleware_count, int max_middleware) {
    if (!node) return;
    
    switch (node->type) {
        case AST_PROGRAM:
            for (int i = 0; i < node->data.program.statement_count; i++) {
                collect_middleware_names_from_ast(node->data.program.statements[i], middleware_names, middleware_count, max_middleware);
            }
            break;
            
        case AST_ROUTE_DEFINITION: {
            // Collect middleware from the main pipeline
            PipelineStep *step = node->data.route_def.pipeline;
            while (step) {
                // Skip "result" as it's built-in
                if (strcmp(step->middleware, "result") != 0) {
                    // Check if middleware is already in the list
                    bool found = false;
                    for (int i = 0; i < *middleware_count; i++) {
                        if (strcmp(middleware_names[i], step->middleware) == 0) {
                            found = true;
                            break;
                        }
                    }
                    if (!found && *middleware_count < max_middleware) {
                        middleware_names[*middleware_count] = strdup(step->middleware);
                        (*middleware_count)++;
                    }
                }
                
                // If this is a result step, collect middleware from its conditions
                if (strcmp(step->middleware, "result") == 0) {
                    ASTNode *result_node = (ASTNode*)(uintptr_t)step->value;
                    ResultCondition *condition = result_node->data.result_step.conditions;
                    while (condition) {
                        PipelineStep *condition_step = condition->pipeline;
                        while (condition_step) {
                            // Skip "result" as it's built-in
                            if (strcmp(condition_step->middleware, "result") != 0) {
                                // Check if middleware is already in the list
                                bool found = false;
                                for (int i = 0; i < *middleware_count; i++) {
                                    if (strcmp(middleware_names[i], condition_step->middleware) == 0) {
                                        found = true;
                                        break;
                                    }
                                }
                                if (!found && *middleware_count < max_middleware) {
                                    middleware_names[*middleware_count] = strdup(condition_step->middleware);
                                    (*middleware_count)++;
                                }
                            }
                            condition_step = condition_step->next;
                        }
                        condition = condition->next;
                    }
                }
                
                step = step->next;
            }
            break;
        }
            
        case AST_VARIABLE_ASSIGNMENT:
            // Variable assignments don't contain pipeline steps
            break;
            
        case AST_RESULT_STEP:
            // This case is handled in the route definition case
            break;
            
        case AST_PIPELINE_STEP:
            // This case is not used in the current implementation
            break;
    }
}

// Runtime initialization
int wp_runtime_init(const char *wp_file) {
    printf("Initializing runtime\n");
    
    // Check if we can access microhttpd functions
    printf("Checking microhttpd availability...\n");
    
    runtime = malloc(sizeof(WPRuntime));
    runtime->middleware = NULL;
    runtime->middleware_count = 0;
    runtime->parse_ctx = parse_context_create();
    if (!runtime->parse_ctx) {
        fprintf(stderr, "Error: Could not create parse context\n");
        free(runtime);
        return -1;
    }
    
    // Use runtime arena for JSON variables
    set_current_arena(runtime->parse_ctx->runtime_arena);
    
    // Set up jansson to use arena allocators once at startup
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);

    runtime->variables = json_object();

    // Parse wp file
    FILE *file = fopen(wp_file, "r");
    if (!file) {
        fprintf(stderr, "Error: Could not open file '%s'\n", wp_file);
        return -1;
    }
    
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    if (file_size <= 0) {
        fprintf(stderr, "Error: File '%s' is empty or invalid\n", wp_file);
        fclose(file);
        return -1;
    }
    
    char *source = malloc((size_t)file_size + 1);
    fread(source, 1, (size_t)file_size, file);
    source[file_size] = '\0';
    fclose(file);

    printf("Tokenizing and parsing\n");
    
    // Tokenize and parse
    int token_count;
    Token *tokens = lexer_tokenize(source, &token_count);
    Parser *parser = parser_new_with_context(tokens, token_count, runtime->parse_ctx);
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
    
    // Collect and load required middleware from AST
    printf("Analyzing AST for required middleware...\n");
    char *middleware_names[64]; // Max 64 middleware
    int middleware_count = 0;
    
    collect_middleware_names_from_ast(runtime->program, middleware_names, &middleware_count, 64);
    
    printf("Found %d unique middleware in AST\n", middleware_count);
    
    // Load required middleware
    printf("Loading middleware...\n");
    for (int i = 0; i < middleware_count; i++) {
        printf("Loading middleware: %s\n", middleware_names[i]);
        if (load_middleware(middleware_names[i]) != 0) {
            printf("Warning: Failed to load %s middleware\n", middleware_names[i]);
        } else {
            printf("Loaded %s middleware successfully\n", middleware_names[i]);
        }
        free(middleware_names[i]); // Free the strdup'd name
    }
    
    // Start HTTP server
    printf("Starting HTTP server on port 8080...\n");
    
    // Try to start the daemon with more detailed error handling
    runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                      8080, NULL, NULL,
                                      &handle_request, NULL,
                                      MHD_OPTION_NOTIFY_COMPLETED, request_completed, NULL,
                                      MHD_OPTION_END);
    
    if (!runtime->daemon) {
        fprintf(stderr, "Error starting HTTP server on port 8080\n");
        
        // Try alternative port
        printf("Trying port 8081...\n");
        runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                          8081, NULL, NULL,
                                          &handle_request, NULL,
                                          MHD_OPTION_NOTIFY_COMPLETED, request_completed, NULL,
                                          MHD_OPTION_END);
        
        if (!runtime->daemon) {
            fprintf(stderr, "Error starting HTTP server on port 8081 as well\n");
            fprintf(stderr, "Check if ports are in use or if you have permission to bind to them\n");
            free(source);
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
        
        // Cleanup middleware
        for (int i = 0; i < runtime->middleware_count; i++) {
            dlclose(runtime->middleware[i].handle);
            free(runtime->middleware[i].name);
        }
        free(runtime->middleware);
        
        // Free parse context (this frees ALL parser memory automatically)
        parse_context_destroy(runtime->parse_ctx);
        
        free(runtime);
    }
} 
