#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdbool.h>
#include <pthread.h>

// Arena allocation function types for middleware
typedef void *(*arena_alloc_func)(void *arena, size_t size);
typedef void (*arena_free_func)(void *arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Cache configuration constants
#define CACHE_SIZE 1024
#define MAX_KEY_LENGTH 256
#define MAX_CACHE_SIZE_MB 100

// Cache entry structure
typedef struct CacheEntry {
    char key[MAX_KEY_LENGTH];
    json_t *response;
    time_t timestamp;
    time_t ttl;
    size_t size;                    // Approximate size in bytes
    struct CacheEntry *next;        // For hash table chaining
    struct CacheEntry *lru_prev;    // For LRU list
    struct CacheEntry *lru_next;    // For LRU list
} CacheEntry;

// Cache configuration
typedef struct {
    int default_ttl;                // Default TTL in seconds
    int max_key_length;             // Maximum cache key length
    size_t max_cache_size;          // Maximum cache size in bytes
    bool enabled;                   // Enable/disable caching
    bool case_sensitive;            // Case sensitive path matching
    bool url_encode_values;         // URL encode resolved values
    char *null_placeholder;         // Placeholder for null values
    bool hash_long_keys;            // Hash keys exceeding max length
} CacheConfig;

// Global cache state (protected by mutex)
static CacheEntry *cache_table[CACHE_SIZE];
static CacheEntry *lru_head = NULL;
static CacheEntry *lru_tail = NULL;
static size_t current_cache_size = 0;
static pthread_mutex_t cache_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t allocator_mutex = PTHREAD_MUTEX_INITIALIZER;

// Default configuration
static CacheConfig cache_config = {
    .default_ttl = 300,             // 5 minutes default
    .max_key_length = 256,
    .max_cache_size = MAX_CACHE_SIZE_MB * 1024 * 1024,
    .enabled = true,
    .case_sensitive = false,
    .url_encode_values = false,
    .null_placeholder = "null",
    .hash_long_keys = true
};

// Function prototypes
static unsigned int hash_key(const char *key);
static CacheEntry *cache_get_unsafe(const char *key);
static int cache_set_unsafe(const char *key, json_t *response, int ttl);
static void cache_delete_unsafe(const char *key);
static void cache_cleanup_expired_unsafe(void);
static void lru_move_to_front(CacheEntry *entry);
static void lru_remove(CacheEntry *entry);
static void lru_add_to_front(CacheEntry *entry);
static CacheEntry *lru_remove_tail(void);
static bool is_expired(CacheEntry *entry);
static char *generate_cache_key(json_t *request, const char *custom_key, 
                               json_t *vary_headers, void *arena, 
                               arena_alloc_func alloc_func);
static char *resolve_key_template(const char *template, json_t *request, void *arena, arena_alloc_func alloc_func);
static size_t estimate_json_size(json_t *json);
static json_t *create_heap_copy(json_t *arena_json);

// Public function prototypes (middleware interface)  
CacheEntry *cache_get(const char *key);
void cache_cleanup(void);
int cache_set(const char *key, json_t *response, int ttl);
void cache_delete(const char *key);
void cache_cleanup_expired(void);
int middleware_init(json_t *config);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, json_t *middleware_config, char **contentType, json_t *variables);
void middleware_post_execute(json_t *final_response, void *arena, arena_alloc_func alloc_func, json_t *middleware_config);

// Hash function for cache keys
static unsigned int hash_key(const char *key) {
    unsigned int hash = 5381;
    int c;
    
    while ((c = *key++)) {
        hash = ((hash << 5) + hash) + (unsigned int)c; // hash * 33 + c
    }
    
    return hash % CACHE_SIZE;
}

// Check if cache entry is expired
static bool is_expired(CacheEntry *entry) {
    return (time(NULL) - entry->timestamp) > entry->ttl;
}

// LRU list management
static void lru_move_to_front(CacheEntry *entry) {
    if (entry == lru_head) {
        return; // Already at front
    }
    
    // Remove from current position
    lru_remove(entry);
    
    // Add to front
    lru_add_to_front(entry);
}

static void lru_remove(CacheEntry *entry) {
    if (entry->lru_prev) {
        entry->lru_prev->lru_next = entry->lru_next;
    } else {
        lru_head = entry->lru_next;
    }
    
    if (entry->lru_next) {
        entry->lru_next->lru_prev = entry->lru_prev;
    } else {
        lru_tail = entry->lru_prev;
    }
    
    entry->lru_prev = NULL;
    entry->lru_next = NULL;
}

static void lru_add_to_front(CacheEntry *entry) {
    entry->lru_prev = NULL;
    entry->lru_next = lru_head;
    
    if (lru_head) {
        lru_head->lru_prev = entry;
    } else {
        lru_tail = entry;
    }
    
    lru_head = entry;
}

static CacheEntry *lru_remove_tail(void) {
    if (!lru_tail) {
        return NULL;
    }
    
    CacheEntry *tail_to_remove = lru_tail;
    
    // Update lru_tail before calling lru_remove to avoid confusion
    if (lru_tail->lru_prev) {
        lru_tail = lru_tail->lru_prev;
        lru_tail->lru_next = NULL;
    } else {
        lru_tail = NULL;
        lru_head = NULL;
    }
    
    // Clear the removed entry's pointers
    tail_to_remove->lru_prev = NULL;
    tail_to_remove->lru_next = NULL;
    
    return tail_to_remove;
}

// Thread-safe cache operations
CacheEntry *cache_get(const char *key) {
    pthread_mutex_lock(&cache_mutex);
    CacheEntry *entry = cache_get_unsafe(key);
    pthread_mutex_unlock(&cache_mutex);
    return entry;
}

int cache_set(const char *key, json_t *response, int ttl) {
    pthread_mutex_lock(&cache_mutex);
    int result = cache_set_unsafe(key, response, ttl);
    pthread_mutex_unlock(&cache_mutex);
    return result;
}

void cache_delete(const char *key) {
    pthread_mutex_lock(&cache_mutex);
    cache_delete_unsafe(key);
    pthread_mutex_unlock(&cache_mutex);
}

void cache_cleanup_expired(void) {
    pthread_mutex_lock(&cache_mutex);
    cache_cleanup_expired_unsafe();
    pthread_mutex_unlock(&cache_mutex);
}

// Unsafe (non-locking) cache operations - must be called with mutex held
static CacheEntry *cache_get_unsafe(const char *key) {
    unsigned int index = hash_key(key);
    CacheEntry *entry = cache_table[index];
    
    while (entry) {
        if (strcmp(entry->key, key) == 0) {
            if (is_expired(entry)) {
                // Remove expired entry
                cache_delete_unsafe(key);
                return NULL;
            }
            
            // Move to front of LRU list
            lru_move_to_front(entry);
            return entry;
        }
        entry = entry->next;
    }
    
    return NULL;
}

static int cache_set_unsafe(const char *key, json_t *response, int ttl) {
    // Check if entry already exists
    CacheEntry *existing = cache_get_unsafe(key);
    if (existing) {
        // Update existing entry - free old heap-allocated response
        if (existing->response) {
            // Synchronize allocator switching
            pthread_mutex_lock(&allocator_mutex);
            
            // Get current allocators
            json_malloc_t current_malloc;
            json_free_t current_free;  
            json_get_alloc_funcs(&current_malloc, &current_free);
            
            // Switch to heap allocators to properly free the cached response
            json_set_alloc_funcs(malloc, free);
            json_decref(existing->response);
            
            // Restore arena allocators
            json_set_alloc_funcs(current_malloc, current_free);
            
            pthread_mutex_unlock(&allocator_mutex);
        }
        
        // Create heap copy of the new response
        existing->response = create_heap_copy(response);
        if (!existing->response) {
            return 0; // Failed to create heap copy
        }
        
        existing->timestamp = time(NULL);
        existing->ttl = ttl;
        existing->size = estimate_json_size(response);
        lru_move_to_front(existing);
        return 1;
    }
    
    // Create new entry
    CacheEntry *entry = malloc(sizeof(CacheEntry));
    if (!entry) {
        return 0;
    }
    
    // Initialize entry
    strncpy(entry->key, key, MAX_KEY_LENGTH - 1);
    entry->key[MAX_KEY_LENGTH - 1] = '\0';
    
    // Create heap copy of the response
    entry->response = create_heap_copy(response);
    if (!entry->response) {
        free(entry);
        return 0; // Failed to create heap copy
    }
    entry->timestamp = time(NULL);
    entry->ttl = ttl;
    entry->size = estimate_json_size(response);
    entry->lru_prev = NULL;
    entry->lru_next = NULL;
    
    // Check if we need to evict entries to make room
    size_t new_total_size = current_cache_size + entry->size;
    while (new_total_size > cache_config.max_cache_size) {
        CacheEntry *evicted = lru_remove_tail();
        if (!evicted) {
            break; // No more entries to evict
        }
        current_cache_size -= evicted->size;
        new_total_size -= evicted->size;
        
        // Remove from hash table
        unsigned int evict_index = hash_key(evicted->key);
        CacheEntry *prev = NULL;
        CacheEntry *curr = cache_table[evict_index];
        
        while (curr && curr != evicted) {
            prev = curr;
            curr = curr->next;
        }
        
        if (curr == evicted) {
            if (prev) {
                prev->next = evicted->next;
            } else {
                cache_table[evict_index] = evicted->next;
            }
        }
        
        // Free heap-allocated response
        if (evicted->response) {
            // Synchronize allocator switching
            pthread_mutex_lock(&allocator_mutex);
            
            // Get current allocators
            json_malloc_t current_malloc;
            json_free_t current_free;  
            json_get_alloc_funcs(&current_malloc, &current_free);
            
            // Switch to heap allocators to properly free the cached response
            json_set_alloc_funcs(malloc, free);
            json_decref(evicted->response);
            
            // Restore arena allocators
            json_set_alloc_funcs(current_malloc, current_free);
            
            pthread_mutex_unlock(&allocator_mutex);
        }
        free(evicted);
    }
    
    // Add to hash table
    unsigned int index = hash_key(key);
    entry->next = cache_table[index];
    cache_table[index] = entry;
    
    // Add to front of LRU list
    lru_add_to_front(entry);
    
    // Update total cache size
    current_cache_size += entry->size;
    
    return 1;
}

static void cache_delete_unsafe(const char *key) {
    unsigned int index = hash_key(key);
    CacheEntry *prev = NULL;
    CacheEntry *entry = cache_table[index];
    
    while (entry) {
        if (strcmp(entry->key, key) == 0) {
            // Remove from hash table
            if (prev) {
                prev->next = entry->next;
            } else {
                cache_table[index] = entry->next;
            }
            
            // Remove from LRU list
            lru_remove(entry);
            
            // Update total cache size
            current_cache_size -= entry->size;
            
            // Free heap-allocated response
            if (entry->response) {
                // Synchronize allocator switching
                pthread_mutex_lock(&allocator_mutex);
                
                // Get current allocators
                json_malloc_t current_malloc;
                json_free_t current_free;  
                json_get_alloc_funcs(&current_malloc, &current_free);
                
                // Switch to heap allocators to properly free the cached response
                json_set_alloc_funcs(malloc, free);
                json_decref(entry->response);
                
                // Restore arena allocators
                json_set_alloc_funcs(current_malloc, current_free);
                
                pthread_mutex_unlock(&allocator_mutex);
            }
            free(entry);
            return;
        }
        prev = entry;
        entry = entry->next;
    }
}

static void cache_cleanup_expired_unsafe(void) {
    time_t now = time(NULL);
    
    for (int i = 0; i < CACHE_SIZE; i++) {
        CacheEntry *entry = cache_table[i];
        CacheEntry *prev = NULL;
        
        while (entry) {
            if ((now - entry->timestamp) > entry->ttl) {
                // Remove expired entry
                CacheEntry *to_remove = entry;
                entry = entry->next;
                
                if (prev) {
                    prev->next = to_remove->next;
                } else {
                    cache_table[i] = to_remove->next;
                }
                
                // Remove from LRU list
                lru_remove(to_remove);
                
                // Update total cache size
                current_cache_size -= to_remove->size;
                
                // Free heap-allocated response
                if (to_remove->response) {
                    // Synchronize allocator switching
                    pthread_mutex_lock(&allocator_mutex);
                    
                    // Get current allocators
                    json_malloc_t current_malloc;
                    json_free_t current_free;  
                    json_get_alloc_funcs(&current_malloc, &current_free);
                    
                    // Switch to heap allocators to properly free the cached response
                    json_set_alloc_funcs(malloc, free);
                    json_decref(to_remove->response);
                    
                    // Restore arena allocators
                    json_set_alloc_funcs(current_malloc, current_free);
                    
                    pthread_mutex_unlock(&allocator_mutex);
                }
                free(to_remove);
            } else {
                prev = entry;
                entry = entry->next;
            }
        }
    }
}

// Estimate JSON object size in bytes without serialization
static size_t estimate_json_size(json_t *json) {
    if (!json) {
        return 0;
    }
    
    size_t size = 0;
    
    switch (json_typeof(json)) {
        case JSON_OBJECT: {
            size += 2; // "{}"
            const char *key;
            json_t *value;
            size_t count = 0;
            json_object_foreach(json, key, value) {
                if (count > 0) size += 1; // comma
                size += strlen(key) + 3; // "key":
                size += estimate_json_size(value);
                count++;
            }
            break;
        }
        case JSON_ARRAY: {
            size += 2; // "[]"
            size_t index;
            json_t *value;
            json_array_foreach(json, index, value) {
                if (index > 0) size += 1; // comma
                size += estimate_json_size(value);
            }
            break;
        }
        case JSON_STRING: {
            const char *str = json_string_value(json);
            size += strlen(str) + 2; // quotes
            break;
        }
        case JSON_INTEGER:
            size += 20; // Max digits for 64-bit int
            break;
        case JSON_REAL:
            size += 32; // Reasonable estimate for double
            break;
        case JSON_TRUE:
            size += 4; // "true"
            break;
        case JSON_FALSE:
            size += 5; // "false"
            break;
        case JSON_NULL:
            size += 4; // "null"
            break;
    }
    
    return size;
}

// Create a heap-allocated copy of arena-allocated JSON
static json_t *create_heap_copy(json_t *arena_json) {
    if (!arena_json) {
        return NULL;
    }
    
    // Synchronize allocator switching across all threads
    pthread_mutex_lock(&allocator_mutex);
    
    // Get current allocators
    json_malloc_t current_malloc;
    json_free_t current_free;  
    json_get_alloc_funcs(&current_malloc, &current_free);
    
    // Switch to heap allocators
    json_set_alloc_funcs(malloc, free);
    json_t *heap_copy = json_deep_copy(arena_json);
    
    // Restore arena allocators
    json_set_alloc_funcs(current_malloc, current_free);
    
    pthread_mutex_unlock(&allocator_mutex);
    
    return heap_copy;
}

// Resolve template variables in cache key template
// Supports simple variable substitution like: user-{params.id}-{query.type}
static char *resolve_key_template(const char *template, json_t *request, void *arena, arena_alloc_func alloc_func) {
    if (!template || !request) {
        return NULL;
    }
    
    size_t template_len = strlen(template);
    size_t result_size = template_len * 2; // Start with double size for expansion
    char *result = alloc_func(arena, result_size);
    if (!result) {
        return NULL;
    }
    
    size_t result_pos = 0;
    size_t i = 0;
    
    while (i < template_len) {
        if (template[i] == '{') {
            // Find the end of the variable
            size_t var_start = i + 1;
            size_t var_end = var_start;
            
            while (var_end < template_len && template[var_end] != '}') {
                var_end++;
            }
            
            if (var_end < template_len) {
                // Extract variable name
                size_t var_name_len = var_end - var_start;
                char *var_name = alloc_func(arena, var_name_len + 1);
                if (var_name) {
                    memcpy(var_name, &template[var_start], var_name_len);
                    var_name[var_name_len] = '\0';
                    
                    // Resolve variable value from JSON
                    json_t *value = NULL;
                    
                    // Handle dot notation like "params.id" or "query.type"
                    char *dot = strchr(var_name, '.');
                    if (dot) {
                        *dot = '\0';
                        char *object_name = var_name;
                        char *property_name = dot + 1;
                        
                        json_t *object = json_object_get(request, object_name);
                        if (object && json_is_object(object)) {
                            value = json_object_get(object, property_name);
                        }
                    } else {
                        // Simple property lookup
                        value = json_object_get(request, var_name);
                    }
                    
                    // Convert value to string and append
                    if (value) {
                        const char *str_value = NULL;
                        char num_buffer[32];
                        
                        if (json_is_string(value)) {
                            str_value = json_string_value(value);
                        } else if (json_is_integer(value)) {
                            snprintf(num_buffer, sizeof(num_buffer), "%lld", json_integer_value(value));
                            str_value = num_buffer;
                        } else if (json_is_real(value)) {
                            snprintf(num_buffer, sizeof(num_buffer), "%.6f", json_real_value(value));
                            str_value = num_buffer;
                        } else if (json_is_boolean(value)) {
                            str_value = json_boolean_value(value) ? "true" : "false";
                        } else if (json_is_null(value)) {
                            str_value = cache_config.null_placeholder;
                        }
                        
                        if (str_value) {
                            size_t str_len = strlen(str_value);
                            // Ensure we have enough space
                            if (result_pos + str_len >= result_size) {
                                result_size = (result_pos + str_len + 1) * 2;
                                char *new_result = alloc_func(arena, result_size);
                                if (new_result) {
                                    memcpy(new_result, result, result_pos);
                                    result = new_result;
                                }
                            }
                            
                            if (result_pos + str_len < result_size) {
                                memcpy(&result[result_pos], str_value, str_len);
                                result_pos += str_len;
                            }
                        }
                    } else {
                        // Variable not found, use placeholder
                        const char *placeholder = cache_config.null_placeholder;
                        size_t placeholder_len = strlen(placeholder);
                        if (result_pos + placeholder_len < result_size) {
                            memcpy(&result[result_pos], placeholder, placeholder_len);
                            result_pos += placeholder_len;
                        }
                    }
                }
                
                i = var_end + 1; // Skip past the '}'
            } else {
                // Unclosed variable, treat as literal
                result[result_pos++] = template[i++];
            }
        } else {
            // Regular character, copy as-is
            if (result_pos < result_size - 1) {
                result[result_pos++] = template[i];
            }
            i++;
        }
    }
    
    result[result_pos] = '\0';
    return result;
}

// Generate cache key from request
static char *generate_cache_key(json_t *request, const char *custom_key, 
                               json_t *vary_headers, void *arena,
                               arena_alloc_func alloc_func) {
    (void)vary_headers; // Not used in basic implementation
    if (custom_key) {
        // Use custom key as-is for now (template expansion will be added in phase 2)
        char *key_buf = alloc_func(arena, strlen(custom_key) + 1);
        if (key_buf) {
            memcpy(key_buf, custom_key, strlen(custom_key) + 1);
            return key_buf;
        }
        return NULL;
    }
    
    // Generate default cache key from request
    const char *method = "GET";
    const char *url = "/";
    
    json_t *method_json = json_object_get(request, "method");
    if (method_json && json_is_string(method_json)) {
        method = json_string_value(method_json);
    }
    
    json_t *url_json = json_object_get(request, "url");
    if (url_json && json_is_string(url_json)) {
        url = json_string_value(url_json);
    }
    
    // Basic key format: METHOD:URL
    size_t key_len = strlen(method) + strlen(url) + 2; // +2 for ':' and '\0'
    
    // Add query parameters to key
    json_t *query = json_object_get(request, "query");
    char query_str[256] = "";
    size_t query_str_len = 0;
    if (query && json_object_size(query) > 0) {
        // TODO: Sort query parameters for consistent keys
        const char *key;
        json_t *value;
        json_object_foreach(query, key, value) {
            if (json_is_string(value)) {
                size_t remaining = sizeof(query_str) - query_str_len - 1;
                if (query_str_len > 0 && remaining > 1) {
                    strncat(query_str, "&", remaining);
                    query_str_len = strlen(query_str);
                    remaining = sizeof(query_str) - query_str_len - 1;
                }
                if (remaining > strlen(key) + strlen(json_string_value(value)) + 1) {
                    strncat(query_str, key, remaining);
                    query_str_len = strlen(query_str);
                    remaining = sizeof(query_str) - query_str_len - 1;
                    strncat(query_str, "=", remaining);
                    query_str_len = strlen(query_str);
                    remaining = sizeof(query_str) - query_str_len - 1;
                    strncat(query_str, json_string_value(value), remaining);
                    query_str_len = strlen(query_str);
                }
            }
        }
        key_len += strlen(query_str) + 1; // +1 for '?'
    }
    
    // Allocate key using arena
    char *cache_key = alloc_func(arena, key_len);
    if (!cache_key) {
        return NULL;
    }
    
    // Build key
    if (strlen(query_str) > 0) {
        snprintf(cache_key, key_len, "%s:%s?%s", method, url, query_str);
    } else {
        snprintf(cache_key, key_len, "%s:%s", method, url);
    }
    
    return cache_key;
}

// Middleware initialization function
int middleware_init(json_t *config) {
    if (config) {
        // Parse global cache configuration
        json_t *enabled_json = json_object_get(config, "enabled");
        if (enabled_json && json_is_boolean(enabled_json)) {
            cache_config.enabled = json_boolean_value(enabled_json);
        }
        
        json_t *default_ttl_json = json_object_get(config, "defaultTtl");
        if (default_ttl_json && json_is_integer(default_ttl_json)) {
            cache_config.default_ttl = (int)json_integer_value(default_ttl_json);
        }
        
        json_t *max_cache_size_json = json_object_get(config, "maxCacheSize");
        if (max_cache_size_json && json_is_integer(max_cache_size_json)) {
            cache_config.max_cache_size = (size_t)json_integer_value(max_cache_size_json);
        }
        
        json_t *max_key_length_json = json_object_get(config, "maxKeyLength");
        if (max_key_length_json && json_is_integer(max_key_length_json)) {
            cache_config.max_key_length = (int)json_integer_value(max_key_length_json);
        }
    }
    
    // Initialize cache table
    for (int i = 0; i < CACHE_SIZE; i++) {
        cache_table[i] = NULL;
    }
    
    printf("Cache middleware initialized: enabled=%s, default_ttl=%d, max_size=%zu MB\n",
           cache_config.enabled ? "true" : "false",
           cache_config.default_ttl,
           cache_config.max_cache_size / (1024 * 1024));
    
    return 0; // Success
}

// Step configuration structure
typedef struct {
    int ttl;
    bool enabled;
    const char *custom_key;
    const char *key_template;
    json_t *vary_headers;
    const char *enabled_when;
} CacheStepConfig;

// Parse step-specific cache configuration (WebPipe native format)
static CacheStepConfig parse_step_config(const char *config, void *arena, arena_alloc_func alloc_func) {
    CacheStepConfig step_config = {
        .ttl = cache_config.default_ttl,
        .enabled = cache_config.enabled,
        .custom_key = NULL,
        .key_template = NULL,
        .vary_headers = NULL,
        .enabled_when = NULL
    };
    
    if (!config || strlen(config) == 0) {
        return step_config;
    }
    
    // Parse WebPipe native config format (key: value pairs)
    // Create a copy of the config string for parsing
    size_t config_len = strlen(config);
    char *config_copy = alloc_func(arena, config_len + 1);
    if (!config_copy) {
        return step_config;
    }
    memcpy(config_copy, config, config_len);
    config_copy[config_len] = '\0';
    
    // Parse line by line
    char *saveptr = NULL;
    char *line = strtok_r(config_copy, "\n", &saveptr);
    
    while (line) {
        // Skip leading whitespace
        while (*line && (*line == ' ' || *line == '\t')) {
            line++;
        }
        
        // Skip empty lines and comments
        if (*line == '\0' || *line == '#') {
            line = strtok_r(NULL, "\n", &saveptr);
            continue;
        }
        
        // Find the colon separator
        char *colon = strchr(line, ':');
        if (colon) {
            *colon = '\0';
            char *key = line;
            char *value = colon + 1;
            
            // Trim key
            char *key_end = key + strlen(key) - 1;
            while (key_end > key && (*key_end == ' ' || *key_end == '\t')) {
                *key_end = '\0';
                key_end--;
            }
            
            // Trim value
            while (*value && (*value == ' ' || *value == '\t')) {
                value++;
            }
            char *value_end = value + strlen(value) - 1;
            while (value_end > value && (*value_end == ' ' || *value_end == '\t')) {
                *value_end = '\0';
                value_end--;
            }
            
            // Parse specific keys
            if (strcmp(key, "ttl") == 0) {
                step_config.ttl = atoi(value);
            } else if (strcmp(key, "enabled") == 0) {
                step_config.enabled = (strcmp(value, "true") == 0);
            } else if (strcmp(key, "key") == 0) {
                // Remove quotes if present
                if (value[0] == '"' && value[strlen(value) - 1] == '"') {
                    value[strlen(value) - 1] = '\0';
                    value++;
                }
                char *key_copy = alloc_func(arena, strlen(value) + 1);
                if (key_copy) {
                    memcpy(key_copy, value, strlen(value) + 1);
                    step_config.custom_key = key_copy;
                }
            } else if (strcmp(key, "keyTemplate") == 0) {
                // Remove quotes if present
                if (value[0] == '"' && value[strlen(value) - 1] == '"') {
                    value[strlen(value) - 1] = '\0';
                    value++;
                }
                char *template_copy = alloc_func(arena, strlen(value) + 1);
                if (template_copy) {
                    memcpy(template_copy, value, strlen(value) + 1);
                    step_config.key_template = template_copy;
                }
            } else if (strcmp(key, "enabledWhen") == 0) {
                // Remove quotes if present
                if (value[0] == '"' && value[strlen(value) - 1] == '"') {
                    value[strlen(value) - 1] = '\0';
                    value++;
                }
                char *condition_copy = alloc_func(arena, strlen(value) + 1);
                if (condition_copy) {
                    memcpy(condition_copy, value, strlen(value) + 1);
                    step_config.enabled_when = condition_copy;
                }
            }
            // Note: varyHeaders array parsing would be more complex, skip for now
        }
        
        line = strtok_r(NULL, "\n", &saveptr);
    }
    
    return step_config;
}

// Middleware execute function (cache check)
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func,
                          arena_free_func free_func,
                          const char *config,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    (void)free_func;    // Not used
    (void)contentType;  // Cache middleware doesn't change content type
    (void)variables;    // Not used in basic implementation
    (void)middleware_config; // Global config not used for step-specific logic
    
    // Handle NULL input gracefully
    if (!input) {
        return NULL; // Cannot cache NULL input
    }
    
    // Check if caching is globally disabled
    if (!cache_config.enabled) {
        return input; // Continue pipeline
    }
    
    // Parse step-specific configuration
    CacheStepConfig step_config = parse_step_config(config, arena, alloc_func);
    
    // Check if caching is disabled for this step
    if (!step_config.enabled) {
        return input; // Continue pipeline
    }
    
    // Use step-specific TTL or fall back to global default
    int ttl = step_config.ttl;
    
    // Determine cache key (priority: custom_key > key_template > auto-generated)
    const char *custom_key = step_config.custom_key;
    if (!custom_key && step_config.key_template) {
        // Resolve template variables to create custom key
        custom_key = resolve_key_template(step_config.key_template, input, arena, alloc_func);
        if (!custom_key) {
            fprintf(stderr, "Cache middleware: Failed to resolve key template: %s\n", step_config.key_template);
            return input; // Continue pipeline on error
        }
    }
    
    json_t *vary_headers = step_config.vary_headers;
    
    // Generate cache key
    char *cache_key = generate_cache_key(input, custom_key, vary_headers, arena, alloc_func);
    if (!cache_key) {
        fprintf(stderr, "Cache middleware: Failed to generate cache key\n");
        return input; // Continue pipeline on error
    }
    
    // Check for cache hit
    CacheEntry *entry = cache_get(cache_key);
    if (entry) {
        // Cache hit - return special control response
        // Need to create an arena-allocated copy of the heap-allocated cached response
        json_t *arena_copy = json_deep_copy(entry->response);
        if (!arena_copy) {
            // Failed to copy, treat as cache miss
            printf("Failed to create arena copy of cached response, treating as miss\n");
        } else {
            json_t *control_response = json_object();
            json_object_set_new(control_response, "_pipeline_action", json_string("return"));
            json_object_set_new(control_response, "value", arena_copy);
            
            return control_response;
        }
    }
    
    // Cache miss - store cache metadata in the request object for post_execute
    // We'll add special metadata that post_execute can read
    json_t *cache_metadata = json_object();
    json_object_set_new(cache_metadata, "cache_key", json_string(cache_key));
    json_object_set_new(cache_metadata, "cache_ttl", json_integer(ttl));
    json_object_set_new(cache_metadata, "cache_enabled", json_boolean(true));
    
    // Get or create generic metadata object
    json_t *metadata = json_object_get(input, "_metadata");
    if (!metadata) {
        metadata = json_object();
        json_object_set_new(input, "_metadata", metadata);
    }
    
    // Add cache metadata to the generic metadata object
    json_object_set_new(metadata, "cache", cache_metadata);
    
    return input; // Continue pipeline
}

// Post-execute function (cache store) - stores response in cache
void middleware_post_execute(json_t *final_response, void *arena,
                           arena_alloc_func alloc_func,
                           json_t *middleware_config) {
    printf("[DEBUG] cache_middleware_post_execute: Starting, final_response=%p\n", final_response);
    fflush(stdout);
    
    (void)arena;          // Not used in cache storage
    (void)alloc_func;     // Not used in cache storage
    (void)middleware_config; // Not needed with our metadata approach
    
    if (!cache_config.enabled) {
        printf("[DEBUG] cache_middleware_post_execute: Cache not enabled, returning\n");
        fflush(stdout);
        return;
    }
    
    printf("[DEBUG] cache_middleware_post_execute: Cache is enabled\n");
    fflush(stdout);
    
    // Look for metadata in the final response
    json_t *metadata = json_object_get(final_response, "_metadata");
    printf("[DEBUG] cache_middleware_post_execute: metadata=%p\n", metadata);
    fflush(stdout);
    if (!metadata) {
        // No metadata, nothing to cache
        printf("[DEBUG] cache_middleware_post_execute: No metadata, returning\n");
        fflush(stdout);
        return;
    }
    
    // Look for cache metadata specifically
    json_t *cache_metadata = json_object_get(metadata, "cache");
    printf("[DEBUG] cache_middleware_post_execute: cache_metadata=%p\n", cache_metadata);
    fflush(stdout);
    if (!cache_metadata) {
        // No cache metadata, nothing to cache
        printf("[DEBUG] cache_middleware_post_execute: No cache metadata, returning\n");
        fflush(stdout);
        return;
    }
    
    // Check if caching is enabled for this request
    json_t *cache_enabled = json_object_get(cache_metadata, "cache_enabled");
    printf("[DEBUG] cache_middleware_post_execute: cache_enabled=%p\n", cache_enabled);
    fflush(stdout);
    if (!cache_enabled || !json_is_boolean(cache_enabled) || !json_boolean_value(cache_enabled)) {
        printf("[DEBUG] cache_middleware_post_execute: Caching not enabled for this request, returning\n");
        fflush(stdout);
        return;
    }
    
    printf("[DEBUG] cache_middleware_post_execute: Caching enabled for this request\n");
    fflush(stdout);
    
    // Extract cache key and TTL from metadata
    json_t *cache_key_json = json_object_get(cache_metadata, "cache_key");
    json_t *cache_ttl_json = json_object_get(cache_metadata, "cache_ttl");
    
    printf("[DEBUG] cache_middleware_post_execute: cache_key_json=%p, cache_ttl_json=%p\n", cache_key_json, cache_ttl_json);
    fflush(stdout);
    
    if (!cache_key_json || !json_is_string(cache_key_json)) {
        printf("Cache metadata missing or invalid cache_key\n");
        fflush(stdout);
        return;
    }
    
    const char *key_str = json_string_value(cache_key_json);
    printf("[DEBUG] cache_middleware_post_execute: key_str='%s'\n", key_str ? key_str : "NULL");
    fflush(stdout);
    
    int ttl = cache_config.default_ttl;
    
    if (cache_ttl_json && json_is_integer(cache_ttl_json)) {
        ttl = (int)json_integer_value(cache_ttl_json);
    }
    
    printf("[DEBUG] cache_middleware_post_execute: ttl=%d\n", ttl);
    fflush(stdout);
    
    // Make a copy of the key string before deleting metadata
    size_t key_len = strlen(key_str);
    char *key_copy = malloc(key_len + 1);
    printf("[DEBUG] cache_middleware_post_execute: Allocated key_copy=%p, key_len=%zu\n", key_copy, key_len);
    fflush(stdout);
    if (!key_copy) {
        printf("[DEBUG] cache_middleware_post_execute: Failed to allocate key_copy, returning\n");
        fflush(stdout);
        return;
    }
    memcpy(key_copy, key_str, key_len + 1);
    printf("[DEBUG] cache_middleware_post_execute: Copied key, key_copy='%s'\n", key_copy);
    fflush(stdout);
    
    // Remove cache metadata from response before storing
    json_t *clean_response = final_response;
    if (!clean_response) {
        printf("[DEBUG] cache_middleware_post_execute: clean_response is NULL, freeing key_copy and returning\n");
        fflush(stdout);
        free(key_copy);
        return;
    }
    printf("[DEBUG] cache_middleware_post_execute: About to delete _metadata\n");
    fflush(stdout);
    // Remove the entire metadata object to keep cached responses clean
    json_object_del(clean_response, "_metadata");
    printf("[DEBUG] cache_middleware_post_execute: Deleted _metadata\n");
    fflush(stdout);
    
    // Store response in cache (cache_set will make its own deep copy)
    printf("[DEBUG] cache_middleware_post_execute: About to call cache_set\n");
    fflush(stdout);
    cache_set(key_copy, clean_response, ttl);
    printf("[DEBUG] cache_middleware_post_execute: cache_set completed\n");
    fflush(stdout);
    free(key_copy);
    printf("[DEBUG] cache_middleware_post_execute: Freed key_copy\n");
    fflush(stdout);
    // if (result) {
    //     printf("Cached response for key: %s (ttl: %d)\n", key, ttl);
    // } else {
    //     printf("Failed to cache response for key: %s\n", key);
    // }
    
    // Clean up our working copy
    // json_decref(clean_response);
    printf("[DEBUG] cache_middleware_post_execute: Completed successfully\n");
    fflush(stdout);
}

// Clean up the entire cache - free all entries and reset cache state
void cache_cleanup(void) {
    if (!cache_config.enabled) {
        return;
    }
    
    pthread_mutex_lock(&cache_mutex);
    
    // Iterate through all hash table entries
    for (int i = 0; i < CACHE_SIZE; i++) {
        CacheEntry *entry = cache_table[i];
        while (entry) {
            CacheEntry *next = entry->next;
            
            // Free heap-allocated response
            if (entry->response) {
                // Synchronize allocator switching
                pthread_mutex_lock(&allocator_mutex);
                
                // Get current allocators
                json_malloc_t current_malloc;
                json_free_t current_free;  
                json_get_alloc_funcs(&current_malloc, &current_free);
                
                // Switch to heap allocators to properly free the cached response
                json_set_alloc_funcs(malloc, free);
                json_decref(entry->response);
                
                // Restore arena allocators
                json_set_alloc_funcs(current_malloc, current_free);
                
                pthread_mutex_unlock(&allocator_mutex);
            }
            
            // Free the cache entry itself
            free(entry);
            entry = next;
        }
        cache_table[i] = NULL;
    }
    
    // Reset cache state
    current_cache_size = 0;
    lru_head = NULL;
    lru_tail = NULL;
    
    pthread_mutex_unlock(&cache_mutex);
}
