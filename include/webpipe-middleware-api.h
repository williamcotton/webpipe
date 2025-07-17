#ifndef WEBPIPE_MIDDLEWARE_API_H
#define WEBPIPE_MIDDLEWARE_API_H

#include <jansson.h>
#include <stddef.h>
#include <stdbool.h>

// Version information
#define WEBPIPE_MIDDLEWARE_API_VERSION_MAJOR 1
#define WEBPIPE_MIDDLEWARE_API_VERSION_MINOR 0
#define WEBPIPE_MIDDLEWARE_API_VERSION_PATCH 0

// Version check macro
#define WEBPIPE_MIDDLEWARE_API_VERSION \
    ((WEBPIPE_MIDDLEWARE_API_VERSION_MAJOR << 16) | \
     (WEBPIPE_MIDDLEWARE_API_VERSION_MINOR << 8) | \
     WEBPIPE_MIDDLEWARE_API_VERSION_PATCH)

// Arena allocation function types
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Forward declarations
struct DatabaseProvider;

// Database function pointer types
typedef json_t* (*execute_sql_func)(const char* sql, json_t* params, void* arena, arena_alloc_func alloc_func);
typedef struct DatabaseProvider* (*get_database_provider_func)(const char* name);
typedef bool (*has_database_provider_func)(void);
typedef const char* (*get_default_database_provider_name_func)(void);

// Database API structure
typedef struct {
    execute_sql_func execute_sql;
    get_database_provider_func get_database_provider;
    has_database_provider_func has_database_provider;
    get_default_database_provider_name_func get_default_database_provider_name;
} WebpipeDatabaseAPI;

// Global database API (initialized by core server)
extern WebpipeDatabaseAPI webpipe_db_api;

// Convenience macros for cleaner code
#define execute_sql(sql, params, arena, alloc_func) \
    webpipe_db_api.execute_sql(sql, params, arena, alloc_func)

#define get_database_provider(name) \
    webpipe_db_api.get_database_provider(name)

#define has_database_provider() \
    webpipe_db_api.has_database_provider()

#define get_default_database_provider_name() \
    webpipe_db_api.get_default_database_provider_name()

// Helper function to check if database API is available
static inline bool webpipe_database_available(void) {
    return webpipe_db_api.execute_sql != NULL;
}

// Helper function to check API version compatibility
static inline bool webpipe_check_api_version(int required_version) {
    return WEBPIPE_MIDDLEWARE_API_VERSION >= required_version;
}

// Standard middleware interface (for reference)
typedef json_t* (*middleware_execute_func)(
    json_t *input, 
    void *arena, 
    arena_alloc_func alloc_func, 
    arena_free_func free_func, 
    const char *config,
    json_t *middleware_config, 
    char **contentType, 
    json_t *variables
);

// Helper function to create standardized errors
static inline json_t *webpipe_create_error(const char *type, const char *message, const char *context) {
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string(type));
    json_object_set_new(error_detail, "message", json_string(message));
    if (context) {
        json_object_set_new(error_detail, "context", json_string(context));
    }
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
}

// Helper function to create auth errors
static inline json_t *webpipe_create_auth_error(const char *message, const char *context) {
    return webpipe_create_error("authError", message, context);
}

// Helper function to create database errors
static inline json_t *webpipe_create_database_error(const char *message, const char *context) {
    return webpipe_create_error("databaseError", message, context);
}

// Helper function to create validation errors
static inline json_t *webpipe_create_validation_error(const char *message, const char *field) {
    return webpipe_create_error("validationError", message, field);
}

// Helper function to check if response contains errors
static inline bool webpipe_has_errors(json_t *response) {
    if (!response) return false;
    json_t *errors = json_object_get(response, "errors");
    return errors && json_is_array(errors) && json_array_size(errors) > 0;
}

// Helper function to get error type from response
static inline const char *webpipe_get_error_type(json_t *response) {
    if (!webpipe_has_errors(response)) return NULL;
    
    json_t *errors = json_object_get(response, "errors");
    json_t *first_error = json_array_get(errors, 0);
    if (!first_error) return NULL;
    
    json_t *type = json_object_get(first_error, "type");
    return type ? json_string_value(type) : NULL;
}

// Helper function to get error message from response
static inline const char *webpipe_get_error_message(json_t *response) {
    if (!webpipe_has_errors(response)) return NULL;
    
    json_t *errors = json_object_get(response, "errors");
    json_t *first_error = json_array_get(errors, 0);
    if (!first_error) return NULL;
    
    json_t *message = json_object_get(first_error, "message");
    return message ? json_string_value(message) : NULL;
}

// Macro to simplify database availability checks
#define WEBPIPE_REQUIRE_DATABASE(error_return) \
    do { \
        if (!webpipe_database_available()) { \
            return webpipe_create_database_error("Database not available", "middleware"); \
        } \
    } while(0)

// Macro to simplify parameter validation
#define WEBPIPE_REQUIRE_PARAM(param, param_name) \
    do { \
        if (!(param)) { \
            return webpipe_create_validation_error("Missing required parameter: " param_name, param_name); \
        } \
    } while(0)

#endif // WEBPIPE_MIDDLEWARE_API_H
