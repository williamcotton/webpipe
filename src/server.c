#include <microhttpd.h>
#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <sys/stat.h>
#include <stdbool.h>
#include <pthread.h>
#include <ctype.h>
#include <limits.h>
#include "wp.h"
#include "database_registry.h"

// Global runtime instance
WPRuntime *runtime = NULL;

// Mutex for protecting jansson allocator function changes during serialization
static pthread_mutex_t serialization_mutex = PTHREAD_MUTEX_INITIALIZER;

// Configuration Management Functions

// Count configuration blocks in AST
static int config_count_blocks(ASTNode *program) {
    if (!program || program->type != AST_PROGRAM) {
        return 0;
    }
    
    int count = 0;
    for (int i = 0; i < program->data.program.statement_count; i++) {
        if (program->data.program.statements[i]->type == AST_CONFIG_BLOCK) {
            count++;
        }
    }
    return count;
}

// Initialize configuration system
static int config_init(ASTNode *program) {
    if (!runtime) {
        return -1;
    }
    
    int config_count = config_count_blocks(program);
    if (config_count == 0) {
        runtime->config_blocks = NULL;
        runtime->config_count = 0;
        return 0;
    }
    
    // Allocate config blocks
    runtime->config_blocks = malloc(sizeof(ConfigBlock) * (size_t)config_count);
    if (!runtime->config_blocks) {
        fprintf(stderr, "Failed to allocate memory for config blocks\n");
        return -1;
    }
    runtime->config_count = 0;
    
    // Extract configuration blocks
    for (int i = 0; i < program->data.program.statement_count; i++) {
        ASTNode *stmt = program->data.program.statements[i];
        if (stmt->type == AST_CONFIG_BLOCK) {
            runtime->config_blocks[runtime->config_count].name = strdup(stmt->data.config_block.name);
            runtime->config_blocks[runtime->config_count].config_json = config_block_to_json(stmt);
            runtime->config_count++;
        }
    }
    return 0;
}

// Get middleware configuration by name
static json_t *config_get(const char *middleware_name) {
    if (!runtime || !runtime->config_blocks || !middleware_name) {
        return NULL;
    }
    
    for (int i = 0; i < runtime->config_count; i++) {
        if (strcmp(runtime->config_blocks[i].name, middleware_name) == 0) {
            return runtime->config_blocks[i].config_json;
        }
    }
    return NULL;
}

// Cleanup configuration system
static void config_cleanup(void) {
    if (!runtime || !runtime->config_blocks) {
        return;
    }
    
    for (int i = 0; i < runtime->config_count; i++) {
        free(runtime->config_blocks[i].name);
    }
    free(runtime->config_blocks);
    runtime->config_blocks = NULL;
    runtime->config_count = 0;
}

// Legacy wrapper for backward compatibility
static void process_config_blocks(ASTNode *program) {
    config_init(program);
}

// Legacy wrapper for backward compatibility  
json_t *get_middleware_config(const char *middleware_name) {
    return config_get(middleware_name);
}

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

// Thread-local storage for post-execute hook registry
static _Thread_local PostExecuteHook *hook_registry = NULL;

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
void *arena_alloc_wrapper(void *arena, size_t size) {
    return arena_alloc((MemoryArena*)arena, size);
}

void arena_free_wrapper(void *arena) {
    arena_free((MemoryArena*)arena);
}

// Hook registry system implementation
void register_post_execute_hook(post_execute_func func, json_t *middleware_config, MemoryArena *arena) {
    if (!func) {
        return;
    }
    
    // Allocate hook entry using the arena
    PostExecuteHook *hook = arena_alloc(arena, sizeof(PostExecuteHook));
    if (!hook) {
        fprintf(stderr, "Failed to allocate memory for post-execute hook\n");
        return;
    }
    
    hook->func = func;
    hook->middleware_config = middleware_config; // Reference, not copied
    hook->next = hook_registry; // Add to front of list
    hook_registry = hook;
}

void execute_post_hooks(json_t *final_response, MemoryArena *arena) {
    PostExecuteHook *current = hook_registry;
    
    // Execute hooks in reverse order (LIFO - last registered, first executed)
    while (current) {
        if (current->func) {
            current->func(final_response, arena, arena_alloc_wrapper, 
                         current->middleware_config);
        }
        current = current->next;
    }
}

void clear_post_hooks(void) {
    // Since hooks are allocated in the request arena, 
    // we just need to clear the registry pointer
    hook_registry = NULL;
}

// Middleware loading and management
// Middleware Loading Helper Functions

// Validate middleware path and name
static int validate_middleware_path(const char *name, char *path_buffer, size_t buffer_size) {
    if (!name || strlen(name) == 0) {
        fprintf(stderr, "Error: Empty middleware name\n");
        return -1;
    }
    
    // Check for path traversal attempts
    if (strstr(name, "..") || strstr(name, "/") || strstr(name, "\\")) {
        fprintf(stderr, "Error: Invalid characters in middleware name: %s\n", name);
        return -1;
    }
    
    // Check if middleware name would cause buffer overflow
    if (strlen(name) > 240) {  // Reserve space for "./middleware/" and ".so\0"
        fprintf(stderr, "Error: Middleware name too long: %s\n", name);
        return -1;
    }
    
    snprintf(path_buffer, buffer_size, "./middleware/%s.so", name);
    return 0;
}

// Resolve required and optional middleware symbols
static int resolve_middleware_symbols(void *handle, const char *name, Middleware *middleware) {
    // Get required middleware execute function
    json_t *(*execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = 
        (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))dlsym(handle, "middleware_execute");
    if (!execute) {
        fprintf(stderr, "Error getting middleware_execute for %s: %s\n", name, dlerror());
        return -1;
    }
    
    // Get optional post_execute function
    post_execute_func post_execute_func_ptr = (post_execute_func)dlsym(handle, "middleware_post_execute");
    // No error checking needed - post_execute is optional
    
    middleware->execute = execute;
    middleware->post_execute = post_execute_func_ptr;
    return 0;
}

// Register middleware as database provider if applicable
static void register_database_provider_if_applicable(void *handle, const char *name) {
    // Check if this middleware is a database provider
    json_t *(*execute_sql_func)(const char *, json_t *, void *, arena_alloc_func) = 
        (json_t *(*)(const char *, json_t *, void *, arena_alloc_func))dlsym(handle, "execute_sql");
    
    if (execute_sql_func) {
        // Register as database provider
        if (register_database_provider(name, handle, execute_sql_func) == 0) {
            printf("Registered database provider: %s\n", name);
        } else {
            printf("Warning: Failed to register database provider: %s\n", name);
        }
    }
    
    // Check if this middleware uses the database API
    typedef struct {
        json_t* (*execute_sql)(const char* sql, json_t* params, void* arena, arena_alloc_func alloc_func);
        DatabaseProvider* (*get_database_provider)(const char* name);
        bool (*has_database_provider)(void);
        const char* (*get_default_database_provider_name)(void);
    } WebpipeDatabaseAPI;
    
    WebpipeDatabaseAPI *db_api_ptr = (WebpipeDatabaseAPI*)dlsym(handle, "webpipe_db_api");
    
    if (db_api_ptr) {
        // Inject database functions into middleware
        db_api_ptr->execute_sql = execute_sql;
        db_api_ptr->get_database_provider = get_database_provider;
        db_api_ptr->has_database_provider = has_database_provider;
        db_api_ptr->get_default_database_provider_name = get_default_database_provider_name;
        
        printf("Injected database API into middleware: %s\n", name);
    }
}

// Initialize middleware if it has an init function
static int initialize_middleware(void *handle, const char *name) {
    int (*init_func)(json_t *config) = (int (*)(json_t *))dlsym(handle, "middleware_init");
    
    if (init_func) {
        json_t *middleware_config = get_middleware_config(name);
        int init_result = init_func(middleware_config);
        
        if (init_result != 0) {
            printf("Warning: Middleware '%s' initialization failed (returned %d)\n", name, init_result);
            // Continue loading despite initialization failure - middleware may still be usable
            return 0; // Don't fail the entire load process
        } else {
            printf("Initialized middleware '%s' successfully\n", name);
        }
    }
    return 0;
}

// Add middleware to runtime registry
static int add_middleware_to_runtime(const char *name, void *handle, const Middleware *middleware) {
    runtime->middleware = realloc(runtime->middleware, sizeof(Middleware) * (size_t)(runtime->middleware_count + 1));
    if (!runtime->middleware) {
        fprintf(stderr, "Error: Failed to allocate memory for middleware registry\n");
        return -1;
    }
    
    runtime->middleware[runtime->middleware_count].name = strdup(name);
    runtime->middleware[runtime->middleware_count].handle = handle;
    runtime->middleware[runtime->middleware_count].execute = middleware->execute;
    runtime->middleware[runtime->middleware_count].post_execute = middleware->post_execute;
    runtime->middleware_count++;
    return 0;
}

// Main middleware loading function
int load_middleware(const char *name) {
    // Check if runtime is initialized
    if (!runtime) {
        fprintf(stderr, "Error: Runtime not initialized\n");
        return -1;
    }
    
    char middleware_path[256];
    if (validate_middleware_path(name, middleware_path, sizeof(middleware_path)) != 0) {
        return -1;
    }
    
    void *handle = dlopen(middleware_path, RTLD_LAZY);
    if (!handle) {
        fprintf(stderr, "Error loading middleware %s: %s\n", name, dlerror());
        return -1;
    }
    
    Middleware middleware = {0};
    if (resolve_middleware_symbols(handle, name, &middleware) != 0) {
        dlclose(handle);
        return -1;
    }
    
    if (add_middleware_to_runtime(name, handle, &middleware) != 0) {
        dlclose(handle);
        return -1;
    }
    
    register_database_provider_if_applicable(handle, name);
    initialize_middleware(handle, name);
    
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

// Helper function to collect query parameters
static enum MHD_Result query_iterator(void *cls, enum MHD_ValueKind kind, const char *key, const char *value) {
    (void)kind; // unused parameter
    json_t *query_obj = (json_t *)cls;
    if (key && value) {
        json_object_set_new(query_obj, key, json_string(value));
    }
    return MHD_YES;
}

// Helper function to collect headers
static enum MHD_Result header_iterator(void *cls, enum MHD_ValueKind kind, const char *key, const char *value) {
    (void)kind; // unused parameter
    json_t *headers_obj = (json_t *)cls;
    if (key && value) {
        json_object_set_new(headers_obj, key, json_string(value));
    }
    return MHD_YES;
}


// HTTP request handling
json_t *create_request_json(struct MHD_Connection *connection, 
                           const char *url, const char *method,
                           PostData *post_data) {
    json_t *request = json_object();
    
    // Set method
    json_object_set_new(request, "method", json_string(method));
    
    // Set URL
    json_object_set_new(request, "url", json_string(url));
    
    // Parse URL params (simple implementation)
    json_t *params = json_object();
    json_object_set_new(request, "params", params);
    
    // Parse query string using MHD
    json_t *query = json_object();
    MHD_get_connection_values(connection, MHD_GET_ARGUMENT_KIND, query_iterator, query);
    json_object_set_new(request, "query", query);
    
    // Parse cookies from Cookie header
    const char *cookie_header = MHD_lookup_connection_value(connection, MHD_HEADER_KIND, "Cookie");
    json_t *cookies = parse_cookies(cookie_header);
    json_object_set_new(request, "cookies", cookies);
    
    // Initialize setCookies array for middleware to add cookies
    json_t *set_cookies = json_array();
    json_object_set_new(request, "setCookies", set_cookies);
    
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
    MHD_get_connection_values(connection, MHD_HEADER_KIND, header_iterator, headers);
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

// Helper function to create standardized error responses
__attribute__((unused)) static json_t *create_error_response(const char *error_type, const char *message) {
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_entry = json_object();
    
    json_object_set_new(error_entry, "type", json_string(error_type));
    json_object_set_new(error_entry, "message", json_string(message));
    
    json_array_append_new(errors_array, error_entry);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
}

// Helper function to create fallback error response in arena
static char *create_fallback_error(const char *message, MemoryArena *arena) {
    // Create a simple JSON error string in arena memory
    const char *template = "{\"error\":\"%s\"}";
    size_t len = strlen(template) + strlen(message) + 1;
    char *error_str = arena_alloc(arena, len);
    if (error_str) {
        snprintf(error_str, len, template, message);
    }
    return error_str;
}

// Helper function to log errors consistently
static void log_error(const char *context, const char *message) {
    fprintf(stderr, "Error in %s: %s\n", context, message);
}

// String processing utilities
typedef struct {
    char **parts;
    int count;
    char *buffer;  // owns the memory
} StringParts;

// Helper function to split string into parts
__attribute__((unused)) static StringParts *split_string_arena(const char *str, const char *delim, MemoryArena *arena, int max_parts) {
    if (!str || !delim || max_parts <= 0) {
        return NULL;
    }
    
    StringParts *sp = arena_alloc(arena, sizeof(StringParts));
    if (!sp) return NULL;
    
    // Create a copy of the string in arena
    sp->buffer = arena_strdup(arena, str);
    if (!sp->buffer) return NULL;
    
    // Allocate array for parts
    sp->parts = arena_alloc(arena, sizeof(char*) * (size_t)max_parts);
    if (!sp->parts) return NULL;
    
    sp->count = 0;
    
    char *saveptr = NULL;
    char *part = strtok_r(sp->buffer, delim, &saveptr);
    
    while (part && sp->count < max_parts) {
        sp->parts[sp->count++] = part;
        part = strtok_r(NULL, delim, &saveptr);
    }
    
    return sp;
}

// Helper function to trim whitespace from string in-place
static char *trim_whitespace(char *str) {
    if (!str) return NULL;
    
    // Skip leading whitespace
    while (*str == ' ' || *str == '\t' || *str == '\n' || *str == '\r') {
        str++;
    }
    
    // Skip trailing whitespace
    char *end = str + strlen(str) - 1;
    while (end > str && (*end == ' ' || *end == '\t' || *end == '\n' || *end == '\r')) {
        *end = '\0';
        end--;
    }
    
    return str;
}

// Helper function to split key-value pairs (like cookies)
static int parse_key_value_pair(char *pair, char **key, char **value, char separator) {
    if (!pair || !key || !value) return -1;
    
    char *sep = strchr(pair, separator);
    if (!sep) return -1;
    
    *sep = '\0';  // Split the string
    *key = trim_whitespace(pair);
    *value = trim_whitespace(sep + 1);
    
    return (strlen(*key) > 0 && strlen(*value) > 0) ? 0 : -1;
}

// Helper function to parse cookies from Cookie header
json_t *parse_cookies(const char *cookie_header) {
    json_t *cookies = json_object();
    if (!cookie_header) {
        return cookies;
    }
    
    // Make a copy of the header for parsing
    char *header_copy = strdup(cookie_header);
    if (!header_copy) {
        return cookies;
    }
    
    // Parse each cookie pair
    char *saveptr = NULL;
    char *cookie_pair = strtok_r(header_copy, ";", &saveptr);
    
    while (cookie_pair) {
        char *key, *value;
        if (parse_key_value_pair(cookie_pair, &key, &value, '=') == 0) {
            json_object_set_new(cookies, key, json_string(value));
        }
        cookie_pair = strtok_r(NULL, ";", &saveptr);
    }
    
    free(header_copy);
    return cookies;
}

// Helper function to clean up response JSON for output (removes internal fields)
json_t *cleanup_response_json(json_t *json_data) {
    if (json_data) {
        // Remove internal pipeline fields that shouldn't be in final response
        json_object_del(json_data, "setCookies");
    }
    return json_data;
}

// Helper function to extract and process cookies from response JSON
static json_t *extract_and_process_cookies(json_t *json_data) {
    json_t *temp_cookies = json_array();
    json_t *set_cookies = json_object_get(json_data, "setCookies");
    
    if (set_cookies && json_is_array(set_cookies)) {
        size_t cookie_count = json_array_size(set_cookies);
        for (size_t i = 0; i < cookie_count; i++) {
            json_t *cookie = json_array_get(set_cookies, i); 
            if (cookie && json_is_string(cookie)) {
                const char *cookie_str = json_string_value(cookie);
                if (cookie_str && strlen(cookie_str) > 0) {
                    json_array_append_new(temp_cookies, json_string(cookie_str));
                }
            }
        }
    }
    
    return temp_cookies;
}

// Helper function to serialize response content based on content type
static char *serialize_response_content(json_t *json_data, const char **content_type, 
                                       size_t *response_len, MemoryArena *arena) {
    char *response_str = NULL;
    
    // Default content type if not specified
    if (!*content_type) {
        *content_type = "application/json";
    }
    
    // Handle different content types
    if (strcmp(*content_type, "application/json") == 0) {
        // JSON response - protect json_dumps from allocator race conditions
        response_str = json_dumps(json_data, JSON_COMPACT);
        if (!response_str) {
            log_error("serialize_response_content", "json_dumps failed");
            response_str = create_fallback_error("JSON serialization failed", arena);
        }
    } else if (strcmp(*content_type, "text/html") == 0 || strcmp(*content_type, "text/plain") == 0) {
        // HTML or text response - extract string from JSON
        if (json_is_string(json_data)) {
            const char *content = json_string_value(json_data);
            if (content) {
                // Use arena_strdup to prevent memory leak
                response_str = arena_strdup(arena, content);
            } else {
                log_error("serialize_response_content", "json_string_value returned NULL");
                response_str = arena_strdup(arena, "Internal server error");
                *content_type = "text/plain";
            }
        } else {
            // Fallback to JSON if not a string
            response_str = json_dumps(json_data, JSON_COMPACT);
            if (!response_str) {
                log_error("serialize_response_content", "json_dumps fallback failed");
                response_str = create_fallback_error("JSON serialization failed", arena);
            }
            *content_type = "application/json";
        }
    } else {
        // Default to JSON for unknown content types
        response_str = json_dumps(json_data, JSON_COMPACT);
        if (!response_str) {
            log_error("serialize_response_content", "json_dumps failed for unknown content type");
            response_str = create_fallback_error("JSON serialization failed", arena);
        }
        *content_type = "application/json";
    }
    
    if (response_str) {
        *response_len = strlen(response_str);
    }
    
    return response_str;
}

// Helper function to create and configure MHD response
static struct MHD_Response *create_http_response(const char *response_str, size_t response_len,
                                                const char *content_type, json_t *cookies) {
    struct MHD_Response *mhd_response = MHD_create_response_from_buffer(
        response_len, (void *)(uintptr_t)response_str, MHD_RESPMEM_PERSISTENT);
    
    if (!mhd_response) {
        return NULL;
    }
    
    // Add cookies to the response
    if (cookies && json_is_array(cookies)) {
        size_t cookie_count = json_array_size(cookies);
        for (size_t i = 0; i < cookie_count; i++) {
            const char *cookie_str = json_string_value(json_array_get(cookies, i));
            if (cookie_str) {
                MHD_add_response_header(mhd_response, "Set-Cookie", cookie_str);
            }
        }
    }
    
    // Add content type header
    MHD_add_response_header(mhd_response, "Content-Type", content_type);
    
    return mhd_response;
}

// Helper function to send response with flexible content type
static enum MHD_Result send_response(struct MHD_Connection *connection, 
                                   json_t *json_data, int status_code, const char *content_type, MemoryArena *arena) {
    char *response_str = NULL;
    size_t response_len = 0;
    
    // Validate inputs
    if (!connection) {
        log_error("send_response", "connection is NULL");
        return MHD_NO;
    }

    // Extract cookies before cleaning up response JSON
    json_t *cookies = extract_and_process_cookies(json_data);

    // Clean up response JSON (removes internal fields like setCookies)
    cleanup_response_json(json_data);
    
    if (!json_data) {
        log_error("send_response", "json_data is NULL");
        // Create a fallback error response
        response_str = create_fallback_error("Internal server error", arena);
        response_len = strlen(response_str);
        content_type = "application/json";
        status_code = 500;
    } else {
        // Serialize response content based on content type
        response_str = serialize_response_content(json_data, &content_type, &response_len, arena);
    }
    
    // Final check that we have a valid response string
    if (!response_str) {
        log_error("send_response", "response_str is still NULL after processing");
        response_str = "{\"error\":\"Critical error\"}";
        response_len = strlen(response_str);
        content_type = "application/json";
    }
    
    // Create HTTP response with cookies and headers
    struct MHD_Response *mhd_response = create_http_response(response_str, response_len, content_type, cookies);
    if (!mhd_response) {
        log_error("send_response", "Failed to create MHD response");
        return MHD_NO;
    }
    
    enum MHD_Result result = MHD_queue_response(connection, (unsigned int)status_code, mhd_response);
    MHD_destroy_response(mhd_response);
    
    return result;
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

// Forward declaration for static file serving
static enum MHD_Result try_serve_static_file(struct MHD_Connection *connection, const char *url, MemoryArena *arena);

// Helper function to find and process matching route
static enum MHD_Result find_and_process_route(struct MHD_Connection *connection,
                                             const char *url, const char *method,
                                             json_t *request, MemoryArena *arena) {
    // NEW: Try static file serving for GET requests
    if (strcmp(method, "GET") == 0) {
        enum MHD_Result static_result = try_serve_static_file(connection, url, arena);
        if (static_result != MHD_NO) {
            return static_result; // File served successfully or error occurred
        }
        // Fall through to route matching if not a static file
    }
    
    // Find matching route
    json_incref(request);
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

/* ───────────────────── static file serving ──────────────────────────────── */

// Forward declarations  
static enum MHD_Result serve_static_file(struct MHD_Connection *connection, const char *file_path, MemoryArena *arena);
static enum MHD_Result try_serve_static_file(struct MHD_Connection *connection, const char *url, MemoryArena *arena);

// MIME type mapping structure
typedef struct {
    const char *extension;
    const char *mime_type;
} MimeMapping;

// MIME type mappings for static files
static MimeMapping mime_types[] = {
    // Web assets
    {".html", "text/html"},
    {".htm", "text/html"},
    {".css", "text/css"},
    {".js", "application/javascript"},
    {".json", "application/json"},
    
    // Images
    {".png", "image/png"},
    {".jpg", "image/jpeg"},
    {".jpeg", "image/jpeg"},
    {".gif", "image/gif"},
    {".svg", "image/svg+xml"},
    {".webp", "image/webp"},
    {".ico", "image/x-icon"},
    {".bmp", "image/bmp"},
    
    // Fonts
    {".woff", "font/woff"},
    {".woff2", "font/woff2"},
    {".ttf", "font/ttf"},
    {".otf", "font/otf"},
    {".eot", "application/vnd.ms-fontobject"},
    
    // Documents
    {".pdf", "application/pdf"},
    {".txt", "text/plain"},
    {".xml", "application/xml"},
    {".csv", "text/csv"},
    
    // Default (must be last)
    {NULL, "application/octet-stream"}
};

// Get MIME type based on file extension
const char *get_mime_type(const char *file_path) {
    if (!file_path) {
        return "application/octet-stream";
    }
    
    // Find the last dot in the filename
    const char *dot = strrchr(file_path, '.');
    if (!dot) {
        return "application/octet-stream";
    }
    
    // Convert extension to lowercase for comparison
    char ext[32];
    strncpy(ext, dot, sizeof(ext) - 1);
    ext[sizeof(ext) - 1] = '\0';
    
    for (char *p = ext; *p; p++) {
        *p = (char)tolower(*p);
    }
    
    // Look up MIME type
    for (int i = 0; mime_types[i].extension; i++) {
        if (strcmp(ext, mime_types[i].extension) == 0) {
            return mime_types[i].mime_type;
        }
    }
    
    return "application/octet-stream";
}

// Validate file access within public directory
int validate_file_access(const char *file_path) {
    struct stat file_stat;
    
    // Check file exists and is readable
    if (stat(file_path, &file_stat) != 0) {
        return -1; // File not found
    }
    
    // Must be regular file (not directory or special file)
    if (!S_ISREG(file_stat.st_mode)) {
        return -1; // Not a regular file
    }
    
    // Resolve symlinks and validate real path is within public/
    char real_path[PATH_MAX];
    if (realpath(file_path, real_path) == NULL) {
        return -1; // Path resolution failed
    }
    
    // Ensure resolved path starts with ./public/
    char public_real[PATH_MAX];
    if (realpath("./public", public_real) == NULL) {
        return -1; // Public directory issue
    }
    
    if (strncmp(real_path, public_real, strlen(public_real)) != 0) {
        return -1; // Path outside public directory
    }
    
    return 0; // File is safe to serve
}

// Validate and sanitize static file path
int validate_static_path(const char *url_path, char *safe_path, size_t path_size) {
    if (!url_path || !safe_path || path_size == 0) {
        return -1;
    }
    
    // Remove leading slash
    if (url_path[0] == '/') {
        url_path++;
    }
    
    // Check for empty path (serve index.html)
    if (strlen(url_path) == 0) {
        snprintf(safe_path, path_size, "./public/index.html");
        return validate_file_access(safe_path);
    }
    
    // Security checks
    if (strstr(url_path, "..") ||      // Path traversal
        strstr(url_path, "\\") ||      // Windows path separators
        url_path[0] == '.' ||          // Hidden files
        strlen(url_path) > 255) {      // Overlong paths
        return -1; // Security violation
    }
    
    // Check for sensitive file extensions
    const char *dangerous_exts[] = {".wp", ".so", ".c", ".h", ".o", ".a", NULL};
    size_t url_len = strlen(url_path);
    for (int i = 0; dangerous_exts[i]; i++) {
        size_t ext_len = strlen(dangerous_exts[i]);
        // Check if the URL ends with this dangerous extension
        if (url_len >= ext_len && 
            strcmp(url_path + url_len - ext_len, dangerous_exts[i]) == 0) {
            return -1; // Sensitive file extension
        }
    }
    
    // Build safe path
    snprintf(safe_path, path_size, "./public/%s", url_path);
    
    // Additional validation
    return validate_file_access(safe_path);
}

// Serve static file content
static enum MHD_Result serve_static_file(struct MHD_Connection *connection, 
                                       const char *file_path, 
                                       MemoryArena *arena) {
    FILE *file = fopen(file_path, "rb");
    if (!file) {
        return MHD_NO; // File not found or not readable
    }
    
    // Get file size
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    // Check file size limit (10MB max)
    const long MAX_FILE_SIZE = 10 * 1024 * 1024;
    if (file_size > MAX_FILE_SIZE) {
        fclose(file);
        return send_error_response(connection,
                                 "{\"error\": \"File too large\"}",
                                 MHD_HTTP_CONTENT_TOO_LARGE);
    }
    
    // Allocate memory for file content using arena
    char *file_content = (char *)arena_alloc(arena, (size_t)file_size);
    if (!file_content) {
        fclose(file);
        return send_error_response(connection,
                                 "{\"error\": \"Memory allocation failed\"}",
                                 MHD_HTTP_INTERNAL_SERVER_ERROR);
    }
    
    // Read file content
    size_t bytes_read = fread(file_content, 1, (size_t)file_size, file);
    fclose(file);
    
    if (bytes_read != (size_t)file_size) {
        return send_error_response(connection,
                                 "{\"error\": \"File read error\"}",
                                 MHD_HTTP_INTERNAL_SERVER_ERROR);
    }
    
    // Get MIME type
    const char *mime_type = get_mime_type(file_path);
    
    // Create HTTP response
    struct MHD_Response *response = MHD_create_response_from_buffer(
        (size_t)file_size, file_content, MHD_RESPMEM_MUST_COPY);
    
    if (!response) {
        return send_error_response(connection,
                                 "{\"error\": \"Response creation failed\"}",
                                 MHD_HTTP_INTERNAL_SERVER_ERROR);
    }
    
    // Set Content-Type header
    MHD_add_response_header(response, "Content-Type", mime_type);
    
    // Set cache headers for static assets
    MHD_add_response_header(response, "Cache-Control", "public, max-age=3600");
    
    // Send response
    enum MHD_Result ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
    MHD_destroy_response(response);
    
    return ret;
}

// Try to serve static file
static enum MHD_Result try_serve_static_file(struct MHD_Connection *connection,
                                           const char *url,
                                           MemoryArena *arena) {
    char safe_path[512];
    
    // Validate and get safe file path
    if (validate_static_path(url, safe_path, sizeof(safe_path)) != 0) {
        // Security violation - return 403 Forbidden
        if (strstr(url, "..") || url[0] == '.' || strstr(url, ".wp") || strstr(url, ".so")) {
            return send_error_response(connection,
                                     "{\"error\": \"Access forbidden\"}",
                                     MHD_HTTP_FORBIDDEN);
        }
        // Not a static file or doesn't exist, continue to route matching
        return MHD_NO;
    }
    
    // File exists and is safe to serve
    return serve_static_file(connection, safe_path, arena);
}

/* ───────────────────── helper utilities ──────────────────────────────── */

static ResultCondition *find_condition_by_name(ResultCondition *c, const char *name) {
    for (; c; c = c->next) {
        if (strcmp(c->condition_name, name) == 0) {
            return c;
        }
    }
    return NULL;
}

static ResultCondition *select_result_condition(ResultCondition *conds, json_t *obj) {
    if (has_errors(obj)) {
        const char *err = get_first_error_type(obj);
        if (err) {
            ResultCondition *match = find_condition_by_name(conds, err);
            if (match) {
                return match;
            }
        }
        ResultCondition *dflt = find_condition_by_name(conds, "default");
        if (dflt) {
            return dflt;
        }
    }
    return find_condition_by_name(conds, "ok");
}


// Merge metadata from previous pipeline step with current step's metadata
static void merge_pipeline_metadata(json_t *result, json_t *previous_step) {
    if (!result || !previous_step) {
        return;
    }
    
    if (!json_is_object(result) || !json_is_object(previous_step)) {
        return;
    }
    
    json_t *previous_metadata = json_object_get(previous_step, "_metadata");
    if (!previous_metadata) {
        return; // No metadata to merge
    }
    
    json_t *current_metadata = json_object_get(result, "_metadata");
    if (!current_metadata) {
        // Result has no metadata, just copy the previous metadata
        json_t *metadata_copy = json_deep_copy(previous_metadata);
        if (metadata_copy) {
            json_object_set(result, "_metadata", metadata_copy);
        }
        return;
    }
    
    // Both have metadata, merge them (current step's metadata takes precedence)
    const char *key;
    json_t *value;
    json_object_foreach(previous_metadata, key, value) {
        if (key && !json_object_get(current_metadata, key)) {
            // Key doesn't exist in current metadata, add it
            json_object_set(current_metadata, key, value);
        }
        // If key exists in current metadata, keep current value (precedence)
    }
}

static inline void attach_request_meta(json_t *dst, json_t *orig) {
    if (!json_is_object(dst) || !orig) return;

    json_object_set(dst, "originalRequest", orig);

    if (!json_object_get(dst, "setCookies")) {
        json_t *sc = json_object_get(orig, "setCookies");
        if (sc) json_object_set(dst, "setCookies", sc);
    }
    
    // Preserve generic metadata if present
    if (!json_object_get(dst, "_metadata")) {
        json_t *metadata = json_object_get(orig, "_metadata");
        if (metadata) json_object_set(dst, "_metadata", metadata);
    }
}

static int dispatch_result(ASTNode *result_node, json_t *state, MemoryArena *arena,
                           json_t **resp, int *code, char **ctype) {
    ResultCondition *cond = select_result_condition(result_node->data.result_step.conditions, state);
    if (!cond) {                       /* no branch found ⇒ 200 OK         */
        *resp = state;
        *code = 200;
        return 0;
    }

    *code = cond->status_code;

    if (!cond->pipeline) {             /* leaf branch                      */
        *resp = state;
        return 0;
    }

    /* inner branch – run its pipeline if present */
    json_t *tmp   = NULL;
    int     tmp_code;
    int ok = execute_pipeline_with_result(cond->pipeline, state, arena,
                                          &tmp, &tmp_code, ctype);
    set_current_arena(arena);

    if (ok == 0 && tmp) {
        state = tmp;                   /* use branch body                  */
    }

    /* keep outer status unless inner pipeline explicitly changed it       */
    if (tmp_code != 200) {
        *code = tmp_code;
    }

    *resp = state;
    return ok;
}

// Helper function to handle result steps
static int handle_result_step(PipelineStep *step, json_t *current, json_t *original_req,
                             MemoryArena *arena, json_t **final_response, 
                             int *response_code, char **content_type) {
    attach_request_meta(current, original_req);
    return dispatch_result((ASTNode *)(uintptr_t)step->value, current,
                          arena, final_response, response_code, content_type);
}

// Helper function to handle pipeline variable steps
static int handle_pipeline_variable(PipelineStep *step, json_t **current, MemoryArena *arena,
                                   char **content_type) {
    json_t *var = json_object_get(runtime->variables, step->value);
    if (!var) {
        log_error("handle_pipeline_variable", step->value);
        return -1;
    }

    json_t *type_field = json_object_get(var, "_type");
    if (!json_is_string(type_field) ||
        strcmp(json_string_value(type_field), "pipeline") != 0) {
        char error_msg[256];
        snprintf(error_msg, sizeof(error_msg), "Variable '%s' is not a pipeline definition", step->value);
        log_error("handle_pipeline_variable", error_msg);
        return -1;
    }

    ASTNode *pnode = (ASTNode *)(uintptr_t)
                     json_integer_value(json_object_get(var, "_definition"));
    if (!pnode || pnode->type != AST_PIPELINE_DEFINITION) {
        char error_msg[256];
        snprintf(error_msg, sizeof(error_msg), "Corrupted pipeline definition for '%s'", step->value);
        log_error("handle_pipeline_variable", error_msg);
        return -1;
    }

    json_t *tmp = NULL;
    char *tmp_ctype = NULL;
    int tmp_code;
    int ok = execute_pipeline_with_result(pnode->data.pipeline_def.pipeline, *current,
                                         arena, &tmp, &tmp_code, &tmp_ctype);
    set_current_arena(arena);
    if (ok != 0) return ok;

    *current = tmp;
    if (tmp_ctype && *content_type && strcmp(tmp_ctype, *content_type) != 0) {
        *content_type = tmp_ctype;
    }
    
    return 0;
}

static json_t *prepare_middleware_input(json_t *current, json_t *original_req, 
                                       const char *variable_name) {
    json_t *middleware_input = current;
    json_incref(middleware_input);
    
    // Ensure originalRequest is available to middleware for template resolution, etc.
    if (!json_object_get(middleware_input, "originalRequest")) {
        json_object_set(middleware_input, "originalRequest", original_req);
    }

    // If using a variable, auto-add resultName (this will be used by the middleware)
    // Note: We remove any existing resultName first since it was for the previous step
    if (variable_name) {
        json_object_del(middleware_input, "resultName");
        json_object_set_new(middleware_input, "resultName", json_string(variable_name));
    }
    
    return middleware_input;
}

// Helper function to handle pipeline control responses (early termination)
static int process_pipeline_control_response(json_t *result, json_t *current, 
                                            json_t *original_req, MemoryArena *arena,
                                            json_t **final_response) {
    json_t *pipeline_action = json_object_get(result, "_pipeline_action");
    if (!pipeline_action || !json_is_string(pipeline_action)) {
        return 0; // No control action
    }
    
    const char *action = json_string_value(pipeline_action);
    if (strcmp(action, "return") == 0) {
        // Early termination - extract value and execute post hooks
        json_t *value = json_object_get(result, "value");
        if (value) {
            // Merge metadata from the current state (which has log start_time)
            // into the cached value before executing post hooks.
            merge_pipeline_metadata(value, current);
            attach_request_meta(value, original_req);
            execute_post_hooks(value, arena);
            json_object_del(value, "originalRequest");

            // Remove empty _metadata object if it exists and is empty
            json_t *metadata = json_object_get(value, "_metadata");
            if (metadata && json_is_object(metadata) && json_object_size(metadata) == 0) {
                json_object_del(value, "_metadata");
            }

            *final_response = value;
            return 1; // Signal early termination
        } else {
            fprintf(stderr, "Pipeline control 'return' without 'value' field\n");
            return -1;
        }
    }
    
    return 0; // No early termination
}

// Helper function to execute a single middleware step
static int execute_middleware_step(Middleware *mw, PipelineStep *step, json_t *middleware_input,
                                  MemoryArena *arena, char **content_type, json_t **result) {
    const char *conf = step->value;
    const char *variable_name = NULL;
    
    if (step->is_variable) {
        variable_name = step->value;  // Store variable name for auto-naming
        json_t *v = json_object_get(runtime->variables, step->value);
        if (v && json_is_string(v)) conf = json_string_value(v);
    }

    json_t *mw_cfg = get_middleware_config(mw->name);
    
    // Check for active mocks in test context
    test_context_t *test_ctx = get_test_context();
    if (test_ctx && is_mock_active(test_ctx, mw->name, variable_name)) {
        *result = get_mock_result(test_ctx, mw->name, variable_name);
        printf("Mock intercepted: %s%s%s\n", mw->name, 
               variable_name ? "." : "", variable_name ? variable_name : "");
    } else {
        // Normal middleware execution
        *result = mw->execute(middleware_input, arena,
                            arena_alloc_wrapper, arena_free_wrapper,
                            conf, mw_cfg, content_type, runtime->variables);
    }
    
    set_current_arena(arena);
    if (!*result) {
        char error_msg[256];
        snprintf(error_msg, sizeof(error_msg), "Middleware %s failed", step->middleware);
        log_error("execute_middleware_step", error_msg);
        return -1;
    }
    
    return 0;
}

// Helper function to handle error cases by looking for result steps
static int handle_error_result(PipelineStep *step, json_t *result, MemoryArena *arena,
                              json_t **final_response, int *response_code, char **content_type) {
    for (PipelineStep *r = step->next; r; r = r->next) {
        if (strcmp(r->middleware, "result") == 0) {
            return dispatch_result((ASTNode *)(uintptr_t)r->value, result,
                                  arena, final_response, response_code, content_type);
        }
    }
    return 0; // No result step found, continue normally
}

// Helper function to merge result into current context
static void merge_step_result(json_t **current, json_t *result, bool is_last_step) {
    if (is_last_step) {
        // Last step: use result as final response (allows clean transformations)
        *current = result;
    } else if (json_is_object(result) && json_is_object(*current)) {
        // Intermediate step: merge the middleware result with the current state
        // Protect JSON operations from allocator race conditions
        pthread_mutex_lock(&serialization_mutex);
        
        json_malloc_t current_malloc;
        json_free_t current_free;
        json_get_alloc_funcs(&current_malloc, &current_free);
        
        json_set_alloc_funcs(malloc, free);
        
        // Create new object to avoid arena/malloc memory conflicts
        json_t *merged = json_object();
        if (!merged) {
            json_set_alloc_funcs(current_malloc, current_free);
            pthread_mutex_unlock(&serialization_mutex);
            *current = result;  // Fallback to replacement
            return;
        }
        
        // First copy all keys from current
        const char *key;
        json_t *value;
        json_object_foreach(*current, key, value) {
            json_object_set(merged, key, value);
        }
        
        // Then copy/overwrite with keys from result
        json_object_foreach(result, key, value) {
            json_object_set(merged, key, value);
        }
        
        json_set_alloc_funcs(current_malloc, current_free);
        pthread_mutex_unlock(&serialization_mutex);
        
        *current = merged;
    } else {
        // Fallback to replacement if either is not an object
        *current = result;
    }
}

/* ───────────────────── main routine ──────────────────────────────────────── */

int execute_pipeline_with_result(PipelineStep *pipeline, json_t *request, MemoryArena *arena,
                                 json_t **final_response, int *response_code, char **content_type) {
    set_current_arena(arena);          /* establish arena context once     */

    // Clear any existing post-execute hooks from previous requests
    clear_post_hooks();

    json_t *current      = request;
    json_t *original_req = request;

    *response_code = 200;
    *content_type  = arena_strdup(arena, "application/json");

    for (PipelineStep *step = pipeline; step; step = step->next) {

        /* ─── result step ─────────────────────────────────────────────── */
        if (strcmp(step->middleware, "result") == 0) {
            return handle_result_step(step, current, original_req, arena, 
                                    final_response, response_code, content_type);
        }

        /* ─── inline pipeline variable ────────────────────────────────── */
        if (strcmp(step->middleware, "pipeline") == 0) {
            int result = handle_pipeline_variable(step, &current, arena, content_type);
            if (result != 0) return result;
            continue;                  /* proceed to next step             */
        }

        /* ─── generic middleware ───────────────────────────────────────── */
        Middleware *mw = find_middleware(step->middleware);
        if (!mw) {
            fprintf(stderr, "Middleware not found: %s\n", step->middleware);
            return -1;
        }

        // Prepare middleware input
        const char *variable_name = step->is_variable ? step->value : NULL;
        json_t *middleware_input = prepare_middleware_input(current, original_req, variable_name);

        json_t *result;
        int exec_result = execute_middleware_step(mw, step, middleware_input, arena, content_type, &result);
        if (exec_result != 0) return exec_result;

        // Check for pipeline control response (early termination)
        int control_result = process_pipeline_control_response(result, current, original_req, arena, final_response);
        if (control_result == 1) return 0;  // Early termination
        if (control_result == -1) return -1; // Error

        attach_request_meta(result, original_req);

        // Merge metadata from the previous pipeline step
        merge_pipeline_metadata(result, current);

        /* ─── register post-execute hooks for middleware that have them ─── */
        if (mw->post_execute) {
            json_t *mw_cfg = get_middleware_config(mw->name);
            register_post_execute_hook(mw->post_execute, mw_cfg, arena);
        }

        /* ─── early‑exit on error ─────────────────────────────────────── */
        if (has_errors(result)) {
            int error_result = handle_error_result(step, result, arena, final_response, response_code, content_type);
            if (error_result != 0) return error_result;
        }

        /* ─── merge result back into current context ──────────────────── */
        bool is_last_step = (step->next == NULL);
        merge_step_result(&current, result, is_last_step);
    }

    /* ─── end of pipeline ──────────────────────────────────────────────── */
    // Execute any registered post-execute hooks
    execute_post_hooks(current, arena);

    // Remove originalRequest after all middleware have had a chance to use it
    json_object_del(current, "originalRequest");

    // Remove empty _metadata object if it exists and is empty
    json_t *metadata = json_object_get(current, "_metadata");
    if (metadata && json_is_object(metadata) && json_object_size(metadata) == 0) {
        json_object_del(current, "_metadata");
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
    
    // Strip query parameters from URL (everything after '?')
    char *query_start = strchr(url_copy, '?');
    if (query_start) {
        *query_start = '\0';
    }
    
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
    
    // Check if pattern has more than 64 parts
    if (pattern_part) {
        fprintf(stderr, "Error: Pattern has more than 64 path segments: %s\n", pattern);
        free(pattern_copy);
        free(url_copy);
        return false;
    }
    
    char *url_part = strtok_r(url_copy, "/", &saveptr2);
    while (url_part && url_count < 64) {
        url_parts[url_count++] = url_part;
        url_part = strtok_r(NULL, "/", &saveptr2);
    }
    
    // Check if URL has more than 64 parts
    if (url_part) {
        fprintf(stderr, "Error: URL has more than 64 path segments: %s\n", url);
        free(pattern_copy);
        free(url_copy);
        return false;
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
                // Skip "result" and "pipeline" as they are built-in
                if (strcmp(step->middleware, "result") != 0 && strcmp(step->middleware, "pipeline") != 0) {
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
                            // Skip "result" and "pipeline" as they are built-in
                            if (strcmp(condition_step->middleware, "result") != 0 && strcmp(condition_step->middleware, "pipeline") != 0) {
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
            
        case AST_PIPELINE_DEFINITION: {
            // Collect middleware from pipeline definition
            PipelineStep *step = node->data.pipeline_def.pipeline;
            while (step) {
                // Skip "result" and "pipeline" as they are built-in
                if (strcmp(step->middleware, "result") != 0 && strcmp(step->middleware, "pipeline") != 0) {
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
                            // Skip "result" and "pipeline" as they are built-in
                            if (strcmp(condition_step->middleware, "result") != 0 && strcmp(condition_step->middleware, "pipeline") != 0) {
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
            
        case AST_RESULT_STEP:
            // This case is handled in the route definition case
            break;
            
        case AST_PIPELINE_STEP:
            // This case is not used in the current implementation
            break;
            
        case AST_CONFIG_BLOCK: {
            // Extract middleware name from config block
            const char *config_name = node->data.config_block.name;
            if (config_name) {
                // Check if middleware is already in the list
                bool found = false;
                for (int i = 0; i < *middleware_count; i++) {
                    if (strcmp(middleware_names[i], config_name) == 0) {
                        found = true;
                        break;
                    }
                }
                if (!found && *middleware_count < max_middleware) {
                    middleware_names[*middleware_count] = strdup(config_name);
                    (*middleware_count)++;
                }
            }
            break;
        }
            
        case AST_CONFIG_VALUE_STRING:
        case AST_CONFIG_VALUE_NUMBER:
        case AST_CONFIG_VALUE_BOOLEAN:
        case AST_CONFIG_VALUE_NULL:
        case AST_CONFIG_VALUE_ENV_CALL:
        case AST_CONFIG_VALUE_OBJECT:
        case AST_CONFIG_VALUE_ARRAY:
            // Configuration values don't contain pipeline steps
            break;
            
        case AST_DESCRIBE_BLOCK:
        case AST_IT_BLOCK:
        case AST_MOCK_CONFIG:
        case AST_TEST_EXECUTION:
        case AST_TEST_ASSERTION:
            // Test nodes don't contain middleware names for loading
            break;
    }
}

// Runtime initialization
int wp_runtime_init(const char *wp_file, int port) {
    printf("Initializing runtime\n");
    
    // Check if we can access microhttpd functions
    printf("Checking microhttpd availability...\n");
    
    runtime = malloc(sizeof(WPRuntime));
    runtime->daemon = NULL;  // Initialize daemon to NULL
    runtime->middleware = NULL;
    runtime->middleware_count = 0;
    runtime->config_blocks = NULL;
    runtime->config_count = 0;
    runtime->parse_ctx = parse_context_create();
    if (!runtime->parse_ctx) {
        fprintf(stderr, "Error: Could not create parse context\n");
        free(runtime);
        return -1;
    }
    
    // Use runtime arena for JSON variables
    set_current_arena(runtime->parse_ctx->runtime_arena);
    
    // Don't use arena allocators for Jansson due to thread safety issues
    // json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);

    // Initialize database registry
    if (database_registry_init() != 0) {
        fprintf(stderr, "Error: Could not initialize database registry\n");
        parse_context_destroy(runtime->parse_ctx);
        free(runtime);
        return -1;
    }

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
    
    // Process configuration blocks
    process_config_blocks(runtime->program);
    
    // Process variable assignments and pipeline definitions
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_VARIABLE_ASSIGNMENT) {
            json_object_set_new(runtime->variables, stmt->data.var_assign.name,
                               json_string(stmt->data.var_assign.value));
        } else if (stmt->type == AST_PIPELINE_DEFINITION) {
            // Store pipeline definitions as a special marker in variables
            // We'll use a JSON object with a special "pipeline" type to identify them
            json_t *pipeline_marker = json_object();
            json_object_set_new(pipeline_marker, "_type", json_string("pipeline"));
            json_object_set_new(pipeline_marker, "_definition", json_integer((json_int_t)(uintptr_t)stmt));
            json_object_set_new(runtime->variables, stmt->data.pipeline_def.name, pipeline_marker);
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
    
    // Check for test mode
    bool has_tests = has_test_blocks(runtime->program);
    bool test_mode = is_test_mode_enabled();
    
    if (test_mode && has_tests) {
        printf("Test mode enabled with test blocks found, executing tests...\n");
        int test_result = execute_test_suite(runtime->program);
        parser_free(parser);
        free_tokens(tokens, token_count);
        free(source);
        return test_result; // Skip HTTP server startup
    } else if (test_mode && !has_tests) {
        printf("Test mode enabled but no test blocks found in %s\n", wp_file);
        parser_free(parser);
        free_tokens(tokens, token_count);
        free(source);
        return -1;
    }
    
    // Start HTTP server
    printf("Starting HTTP server on port %d...\n", port);
    
    // Try to start the daemon with more detailed error handling
    runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                      (uint16_t)port, NULL, NULL,
                                      &handle_request, NULL,
                                      MHD_OPTION_NOTIFY_COMPLETED, request_completed, NULL,
                                      MHD_OPTION_END);
    
    if (!runtime->daemon) {
        fprintf(stderr, "Error starting HTTP server on port %d\n", port);
        fprintf(stderr, "Check if port is in use or if you have permission to bind to it\n");
        free(source);
        return -1;
    } else {
        printf("HTTP server started successfully on port %d\n", port);
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
        
        // Cleanup configuration blocks
        config_cleanup();
        
        // Cleanup database registry
        database_registry_cleanup();
        
        // Free parse context (this frees ALL parser memory automatically)
        parse_context_destroy(runtime->parse_ctx);
        
        free(runtime);
    }
} 
