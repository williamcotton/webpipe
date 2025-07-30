#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>

// Arena allocation function types for middleware
typedef void *(*arena_alloc_func)(void *arena, size_t size);
typedef void (*arena_free_func)(void *arena);

// Public function prototypes (middleware interface)
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, 
                          arena_free_func free_func, const char *config,
                          json_t *middleware_config, char **contentType, json_t *variables);

// Debug middleware execute function - prints input JSON and returns it unchanged
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func,
                          arena_free_func free_func,
                          const char *config,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    (void)arena;            // Not used
    (void)alloc_func;       // Not used
    (void)free_func;        // Not used
    (void)config;           // Not used
    (void)middleware_config; // Not used
    (void)contentType;      // Debug middleware doesn't change content type
    (void)variables;        // Not used
    
    // Print the JSON dump of the input
    if (input) {
        char *json_str = json_dumps(input, JSON_INDENT(2));
        if (json_str) {
            printf("%s\n%s\n", config, json_str);
        } else {
            printf("%s\nFailed to serialize input JSON\n", config);
        }
    } else {
        printf("%s\nInput is NULL\n", config);
    }
    
    // Return the same input unchanged
    return input;
}