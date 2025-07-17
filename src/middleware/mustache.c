#include <jansson.h>
#include <string.h>
#include <stdlib.h>
#include "../../deps/mustach/mustach-jansson.h"
#include "../../deps/mustach/mustach-wrap.h"

// Arena allocation function types for middlewares
typedef void *(*arena_alloc_func)(void *arena, size_t size);
typedef void (*arena_free_func)(void *arena);

// External arena allocator functions
extern void *jansson_arena_malloc(size_t size);
extern void jansson_arena_free(void *ptr);

// Thread-local storage for current variables (during request processing)
static __thread json_t *current_variables = NULL;

// Function to find mustache partials from the variables
static const char *find_partial(json_t *variables, const char *name) {
    if (!variables || !name) {
        return NULL;
    }
    
    json_t *var_value = json_object_get(variables, name);
    
    if (var_value && json_is_string(var_value)) {
        return json_string_value(var_value);
    }
    
    return NULL;
}

// Partial handler callback for mustach library
static int partial_handler(const char *name, struct mustach_sbuf *sbuf) {
    if (!current_variables) {
        return MUSTACH_ERROR_PARTIAL_NOT_FOUND;
    }
    
    const char *template = find_partial(current_variables, name);
    if (!template) {
        return MUSTACH_ERROR_PARTIAL_NOT_FOUND;
    }
    
    sbuf->value = template;
    sbuf->freecb = NULL; // Template is managed by variables
    return MUSTACH_OK;
}

// Local arena_strdup implementation (copied from server.c)
static char *local_arena_strdup(void *arena, arena_alloc_func alloc_func, const char *str) {
    if (!str) return NULL;
    
    size_t len = strlen(str);
    char *copy = alloc_func(arena, len + 1);
    if (copy) {
        memcpy(copy, str, len);
        copy[len] = '\0';
    }
    return copy;
}

// Forward declarations
static char *render_mustache_template(const char *template, json_t *data, void *arena, arena_alloc_func alloc_func);
static json_t *create_template_error(const char *message, const char *template);

// Middleware interface function declaration
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *template,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables);

// Middleware interface function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *template,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    // Suppress unused parameter warnings
    (void)free_func;
    (void)middleware_config;  // Unused parameter for now
    

    
    // Set up partials for this request
    current_variables = variables;
    mustach_wrap_get_partial = partial_handler;
    
    // Render mustache template with input JSON data
    char *html = render_mustache_template(template, input, arena, alloc_func);
    if (!html) {
        // Clean up and DON'T set content type for errors - return JSON error object
        current_variables = NULL;
        return create_template_error("Template rendering failed", template);
    }
    
    // ONLY set content type to HTML on successful render
    *contentType = local_arena_strdup(arena, alloc_func, "text/html");
    
    // Clean up
    current_variables = NULL;
    
    // Return HTML content as JSON string
    return json_string(html);
}

// Template rendering function  
static char *render_mustache_template(const char *template, json_t *data, void *arena, arena_alloc_func alloc_func) {
    char *result = NULL;
    size_t result_size = 0;
    
    // Save current jansson allocators
    json_malloc_t current_malloc;
    json_free_t current_free;
    json_get_alloc_funcs(&current_malloc, &current_free);
    
    // Temporarily switch to standard malloc/free for mustache library
    json_set_alloc_funcs(malloc, free);
    
    // Use mustach_jansson_mem to render template - this will use standard malloc
    int rc = mustach_jansson_mem(template, strlen(template), data,
                                Mustach_With_AllExtensions, &result, &result_size);
    
    // Restore arena allocators for jansson
    json_set_alloc_funcs(current_malloc, current_free);
    
    if (rc != MUSTACH_OK) {
        return NULL; // Return NULL for error, let caller handle
    }
    
    // Copy result from malloc memory to arena memory
    char *arena_result = local_arena_strdup(arena, alloc_func, result);
    
    // Free the malloc'd result since we copied it to arena
    free(result);
    
    return arena_result;
}

// Error handling function
static json_t *create_template_error(const char *message, const char *template) {
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string("templateError"));
    json_object_set_new(error_detail, "message", json_string(message));
    
    // Only include template snippet if not too long
    if (template && strlen(template) < 100) {
        json_object_set_new(error_detail, "template", json_string(template));
    }
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
}
