#include <jansson.h>
#include <string.h>
#include <stdlib.h>
#include "../../deps/mustach/mustach-jansson.h"
#include "../wp.h"

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
                          char **contentType);

// Middleware interface function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *template,
                          char **contentType) {
    // Suppress unused parameter warnings
    (void)free_func;
    
    // Render mustache template with input JSON data
    char *html = render_mustache_template(template, input, arena, alloc_func);
    if (!html) {
        // DON'T set content type for errors - return JSON error object
        return create_template_error("Template rendering failed", template);
    }
    
    // ONLY set content type to HTML on successful render
    *contentType = local_arena_strdup(arena, alloc_func, "text/html");
    
    // Return HTML content as JSON string
    return json_string(html);
}

// Template rendering function
static char *render_mustache_template(const char *template, json_t *data, void *arena, arena_alloc_func alloc_func) {
    char *result = NULL;
    size_t result_size = 0;
    
    // Use mustach_jansson_mem to render template
    int rc = mustach_jansson_mem(template, strlen(template), data,
                                Mustach_With_AllExtensions, &result, &result_size);
    
    if (rc != MUSTACH_OK) {
        return NULL; // Return NULL for error, let caller handle
    }
    
    // Copy result to arena and free original (following old_mustache.c pattern)
    char *arena_result = local_arena_strdup(arena, alloc_func, result);
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
