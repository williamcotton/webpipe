#ifndef DATABASE_REGISTRY_H
#define DATABASE_REGISTRY_H

#include <jansson.h>
#include <pthread.h>
#include <stdbool.h>
#include <stddef.h>

// Forward declarations
typedef struct MemoryArena MemoryArena;

// Arena allocation function types
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Maximum number of database providers
#define MAX_DATABASE_PROVIDERS 16

// Database provider structure
typedef struct DatabaseProvider {
    char *name;                    // "pg", "mysql", etc.
    void *middleware_handle;       // Handle to loaded middleware
    json_t *(*execute_sql)(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func);
    bool is_available;             // Runtime availability status
} DatabaseProvider;

// Global database registry
typedef struct DatabaseRegistry {
    DatabaseProvider *providers[MAX_DATABASE_PROVIDERS];
    int provider_count;
    DatabaseProvider *default_provider;  // First available provider
    pthread_mutex_t mutex;
    bool initialized;
} DatabaseRegistry;

// Core registry functions
int database_registry_init(void);
void database_registry_cleanup(void);
bool database_registry_is_initialized(void);

// Provider registration functions
int register_database_provider(const char *name, void *middleware_handle, 
                              json_t *(*execute_sql_func)(const char *, json_t *, void *, arena_alloc_func));
int unregister_database_provider(const char *name);

// Provider access functions
DatabaseProvider *get_default_database_provider(void);
DatabaseProvider *get_database_provider(const char *name);
bool has_database_provider(void);

// Get the name of the default database provider
const char *get_default_database_provider_name(void);

// Default execute_sql function (uses default provider)
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func);

// Utility functions
char **list_database_provider_names(int *count);

// Registry statistics
typedef struct {
    int total_providers;
    int available_providers;
    char **provider_names;
} DatabaseRegistryStats;

DatabaseRegistryStats *get_database_registry_stats(void);
void free_database_registry_stats(DatabaseRegistryStats *stats);

#endif // DATABASE_REGISTRY_H 
