#include "database_registry.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Global database registry
static DatabaseRegistry db_registry = {0};

// Initialize database registry
int database_registry_init(void) {
    if (db_registry.initialized) {
        return 0; // Already initialized
    }
    
    // Initialize mutex
    if (pthread_mutex_init(&db_registry.mutex, NULL) != 0) {
        fprintf(stderr, "Failed to initialize database registry mutex\n");
        return -1;
    }
    
    // Initialize registry fields
    db_registry.provider_count = 0;
    db_registry.default_provider = NULL;
    db_registry.initialized = true;
    
    // Initialize provider array
    for (int i = 0; i < MAX_DATABASE_PROVIDERS; i++) {
        db_registry.providers[i] = NULL;
    }
    
    printf("Database registry initialized\n");
    return 0;
}

// Cleanup database registry
void database_registry_cleanup(void) {
    if (!db_registry.initialized) {
        return;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    // Free all providers
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i]) {
            free(db_registry.providers[i]->name);
            free(db_registry.providers[i]);
            db_registry.providers[i] = NULL;
        }
    }
    
    db_registry.provider_count = 0;
    db_registry.default_provider = NULL;
    db_registry.initialized = false;
    
    pthread_mutex_unlock(&db_registry.mutex);
    pthread_mutex_destroy(&db_registry.mutex);
    
    printf("Database registry cleanup completed\n");
}

// Check if registry is initialized
bool database_registry_is_initialized(void) {
    return db_registry.initialized;
}

// Register database provider
int register_database_provider(const char *name, void *middleware_handle, 
                              json_t *(*execute_sql_func)(const char *, json_t *, void *, arena_alloc_func)) {
    if (!db_registry.initialized) {
        fprintf(stderr, "Database registry not initialized\n");
        return -1;
    }
    
    if (!name || !execute_sql_func) {
        fprintf(stderr, "Invalid parameters for database provider registration\n");
        return -1;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    // Check if provider already exists
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i] && strcmp(db_registry.providers[i]->name, name) == 0) {
            printf("Database provider '%s' already registered, updating\n", name);
            db_registry.providers[i]->middleware_handle = middleware_handle;
            db_registry.providers[i]->execute_sql = execute_sql_func;
            db_registry.providers[i]->is_available = true;
            pthread_mutex_unlock(&db_registry.mutex);
            return 0;
        }
    }
    
    // Check if we have space for new provider
    if (db_registry.provider_count >= MAX_DATABASE_PROVIDERS) {
        fprintf(stderr, "Maximum number of database providers reached (%d)\n", MAX_DATABASE_PROVIDERS);
        pthread_mutex_unlock(&db_registry.mutex);
        return -1;
    }
    
    // Create new provider
    DatabaseProvider *provider = malloc(sizeof(DatabaseProvider));
    if (!provider) {
        fprintf(stderr, "Failed to allocate memory for database provider\n");
        pthread_mutex_unlock(&db_registry.mutex);
        return -1;
    }
    
    provider->name = strdup(name);
    if (!provider->name) {
        fprintf(stderr, "Failed to duplicate provider name\n");
        free(provider);
        pthread_mutex_unlock(&db_registry.mutex);
        return -1;
    }
    
    provider->middleware_handle = middleware_handle;
    provider->execute_sql = execute_sql_func;
    provider->is_available = true;
    
    // Add to registry
    db_registry.providers[db_registry.provider_count] = provider;
    db_registry.provider_count++;
    
    // Set as default provider if it's the first one
    if (!db_registry.default_provider) {
        db_registry.default_provider = provider;
        printf("Set '%s' as default database provider\n", name);
    }
    
    pthread_mutex_unlock(&db_registry.mutex);
    
    return 0;
}

// Unregister database provider
int unregister_database_provider(const char *name) {
    if (!db_registry.initialized) {
        return -1;
    }
    
    if (!name) {
        return -1;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    // Find and remove provider
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i] && strcmp(db_registry.providers[i]->name, name) == 0) {
            // If this was the default provider, clear it
            if (db_registry.default_provider == db_registry.providers[i]) {
                db_registry.default_provider = NULL;
                // Set new default if other providers exist
                for (int j = 0; j < db_registry.provider_count; j++) {
                    if (j != i && db_registry.providers[j]) {
                        db_registry.default_provider = db_registry.providers[j];
                        break;
                    }
                }
            }
            
            // Free provider memory
            free(db_registry.providers[i]->name);
            free(db_registry.providers[i]);
            
            // Shift remaining providers
            for (int j = i; j < db_registry.provider_count - 1; j++) {
                db_registry.providers[j] = db_registry.providers[j + 1];
            }
            db_registry.providers[db_registry.provider_count - 1] = NULL;
            db_registry.provider_count--;
            
            pthread_mutex_unlock(&db_registry.mutex);
            printf("Unregistered database provider: %s\n", name);
            return 0;
        }
    }
    
    pthread_mutex_unlock(&db_registry.mutex);
    return -1; // Provider not found
}

// Get default database provider
DatabaseProvider *get_default_database_provider(void) {
    if (!db_registry.initialized) {
        return NULL;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    DatabaseProvider *provider = db_registry.default_provider;
    pthread_mutex_unlock(&db_registry.mutex);
    
    return provider;
}

// Get database provider by name
DatabaseProvider *get_database_provider(const char *name) {
    if (!db_registry.initialized || !name) {
        return NULL;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i] && strcmp(db_registry.providers[i]->name, name) == 0) {
            DatabaseProvider *provider = db_registry.providers[i];
            pthread_mutex_unlock(&db_registry.mutex);
            return provider;
        }
    }
    
    pthread_mutex_unlock(&db_registry.mutex);
    return NULL;
}

// Check if any database provider is available
bool has_database_provider(void) {
    pthread_mutex_lock(&db_registry.mutex);
    bool has_provider = (db_registry.provider_count > 0);
    pthread_mutex_unlock(&db_registry.mutex);
    return has_provider;
}

// Get the name of the default database provider
const char *get_default_database_provider_name(void) {
    pthread_mutex_lock(&db_registry.mutex);
    const char *name = NULL;
    if (db_registry.default_provider) {
        name = db_registry.default_provider->name;
    }
    pthread_mutex_unlock(&db_registry.mutex);
    return name;
}

// Default execute_sql function (uses default provider)
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func) {
    if (!db_registry.initialized) {
        // Return standardized error
        json_t *error = json_object();
        json_t *errors_array = json_array();
        json_t *error_detail = json_object();
        
        json_object_set_new(error_detail, "type", json_string("databaseError"));
        json_object_set_new(error_detail, "message", json_string("Database registry not initialized"));
        
        json_array_append_new(errors_array, error_detail);
        json_object_set_new(error, "errors", errors_array);
        
        return error;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    if (!db_registry.default_provider) {
        pthread_mutex_unlock(&db_registry.mutex);
        
        // Return standardized error
        json_t *error = json_object();
        json_t *errors_array = json_array();
        json_t *error_detail = json_object();
        
        json_object_set_new(error_detail, "type", json_string("databaseError"));
        json_object_set_new(error_detail, "message", json_string("No database provider available"));
        
        json_array_append_new(errors_array, error_detail);
        json_object_set_new(error, "errors", errors_array);
        
        return error;
    }
    
    DatabaseProvider *provider = db_registry.default_provider;
    json_t *(*execute_func)(const char *, json_t *, void *, arena_alloc_func) = provider->execute_sql;
    
    pthread_mutex_unlock(&db_registry.mutex);
    
    return execute_func(sql, params, arena, alloc_func);
}

// List database provider names
char **list_database_provider_names(int *count) {
    if (!db_registry.initialized || !count) {
        if (count) *count = 0;
        return NULL;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    if (db_registry.provider_count == 0) {
        *count = 0;
        pthread_mutex_unlock(&db_registry.mutex);
        return NULL;
    }
    
    char **names = malloc(sizeof(char*) * (size_t)db_registry.provider_count);
    if (!names) {
        *count = 0;
        pthread_mutex_unlock(&db_registry.mutex);
        return NULL;
    }
    
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i]) {
            names[i] = strdup(db_registry.providers[i]->name);
            if (!names[i]) {
                // Cleanup on failure
                for (int j = 0; j < i; j++) {
                    free(names[j]);
                }
                free(names);
                *count = 0;
                pthread_mutex_unlock(&db_registry.mutex);
                return NULL;
            }
        } else {
            names[i] = NULL;
        }
    }
    
    *count = db_registry.provider_count;
    pthread_mutex_unlock(&db_registry.mutex);
    
    return names;
}

// Helper function to get provider names without acquiring mutex (assumes mutex is already held)
static char **get_provider_names_unlocked(int *count) {
    if (db_registry.provider_count == 0) {
        *count = 0;
        return NULL;
    }
    
    char **names = malloc(sizeof(char*) * (size_t)db_registry.provider_count);
    if (!names) {
        *count = 0;
        return NULL;
    }
    
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i]) {
            names[i] = strdup(db_registry.providers[i]->name);
            if (!names[i]) {
                // Cleanup on failure
                for (int j = 0; j < i; j++) {
                    free(names[j]);
                }
                free(names);
                *count = 0;
                return NULL;
            }
        } else {
            names[i] = NULL;
        }
    }
    
    *count = db_registry.provider_count;
    return names;
}

// Get database registry statistics
DatabaseRegistryStats *get_database_registry_stats(void) {
    if (!db_registry.initialized) {
        return NULL;
    }
    
    DatabaseRegistryStats *stats = malloc(sizeof(DatabaseRegistryStats));
    if (!stats) {
        return NULL;
    }
    
    pthread_mutex_lock(&db_registry.mutex);
    
    stats->total_providers = db_registry.provider_count;
    stats->available_providers = 0;
    
    // Count available providers
    for (int i = 0; i < db_registry.provider_count; i++) {
        if (db_registry.providers[i] && db_registry.providers[i]->is_available) {
            stats->available_providers++;
        }
    }
    
    // Copy provider names using helper function
    stats->provider_names = get_provider_names_unlocked(&stats->total_providers);
    
    pthread_mutex_unlock(&db_registry.mutex);
    
    return stats;
}

// Free database registry statistics
void free_database_registry_stats(DatabaseRegistryStats *stats) {
    if (!stats) {
        return;
    }
    
    if (stats->provider_names) {
        for (int i = 0; i < stats->total_providers; i++) {
            free(stats->provider_names[i]);
        }
        free(stats->provider_names);
    }
    
    free(stats);
}
