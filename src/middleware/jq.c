#include <jansson.h>
#include <jq.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Function prototype for middleware interface
json_t *middleware_execute(json_t *input, void *arena, void *alloc, void *free_func, const char *filter);

// Hash table for caching
#define HASH_TABLE_SIZE 256
#define HASH_MASK (HASH_TABLE_SIZE - 1)

typedef struct jq_cache_entry {
  char *filter;
  jq_state *jq;
  struct jq_cache_entry *next;
  pthread_mutex_t mutex; // Per-entry mutex for thread safety
} jq_cache_entry;

// Global cache with mutex protection
static jq_cache_entry *jq_cache[HASH_TABLE_SIZE];
static pthread_mutex_t cache_mutex = PTHREAD_MUTEX_INITIALIZER;
static int cache_initialized = 0;

static uint32_t hash_string(const char *str) {
  uint32_t hash = 5381;
  int c;
  while ((c = *str++)) {
    hash = ((hash << 5) + hash) + (uint32_t)c;
  }
  return hash;
}

static void init_cache(void) {
  pthread_mutex_lock(&cache_mutex);
  if (!cache_initialized) {
    memset(jq_cache, 0, sizeof(jq_cache));
    cache_initialized = 1;
  }
  pthread_mutex_unlock(&cache_mutex);
}

static jq_state *get_cached_jq(const char *filter) {
  if (!cache_initialized) {
    init_cache();
  }

  uint32_t hash = hash_string(filter) & HASH_MASK;

  // First, try to find existing entry without holding the global lock
  jq_cache_entry *entry = jq_cache[hash];
  while (entry) {
    if (strcmp(entry->filter, filter) == 0) {
      return entry->jq;
    }
    entry = entry->next;
  }

  // Not found, need to create new entry
  pthread_mutex_lock(&cache_mutex);

  // Check again in case another thread just added it
  entry = jq_cache[hash];
  while (entry) {
    if (strcmp(entry->filter, filter) == 0) {
      pthread_mutex_unlock(&cache_mutex);
      return entry->jq;
    }
    entry = entry->next;
  }

  // Create new entry
  entry = malloc(sizeof(jq_cache_entry));
  entry->filter = strdup(filter);
  entry->jq = jq_init();
  pthread_mutex_init(&entry->mutex, NULL);

  if (!jq_compile(entry->jq, filter)) {
    pthread_mutex_destroy(&entry->mutex);
    jq_teardown(&entry->jq);
    free(entry->filter);
    free(entry);
    pthread_mutex_unlock(&cache_mutex);
    return NULL;
  }

  // Add to cache
  entry->next = jq_cache[hash];
  jq_cache[hash] = entry;

  pthread_mutex_unlock(&cache_mutex);

  return entry->jq;
}

// Thread-local execution state
typedef struct {
  jv value;
  int valid;
} jq_result;

// Minimal conversion functions
static jv json_to_jv(json_t *j) {
  if (!j)
    return jv_null();

  switch (json_typeof(j)) {
  case JSON_NULL:
    return jv_null();
  case JSON_TRUE:
    return jv_true();
  case JSON_FALSE:
    return jv_false();
  case JSON_INTEGER: {
    json_int_t int_val = json_integer_value(j);
    return jv_number((double)int_val);
  }
  case JSON_REAL:
    return jv_number(json_real_value(j));
  case JSON_STRING:
    return jv_string(json_string_value(j));
  case JSON_ARRAY: {
    jv arr = jv_array();
    size_t i;
    json_t *v;
    json_array_foreach(j, i, v) { arr = jv_array_append(arr, json_to_jv(v)); }
    return arr;
  }
  case JSON_OBJECT: {
    jv obj = jv_object();
    const char *k;
    json_t *v;
    json_object_foreach(j, k, v) {
      obj = jv_object_set(obj, jv_string(k), json_to_jv(v));
    }
    return obj;
  }
  }
  return jv_null();
}


// Arena allocation function type
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Arena-aware JSON construction to avoid global allocator conflicts
static json_t *jv_to_json_with_arena(jv v, void *arena, arena_alloc_func alloc_func) {
  switch (jv_get_kind(v)) {
  case JV_KIND_INVALID:
    jv_free(v);
    return NULL;
  case JV_KIND_NULL:
    jv_free(v);
    return json_null();
  case JV_KIND_FALSE:
    jv_free(v);
    return json_false();
  case JV_KIND_TRUE:
    jv_free(v);
    return json_true();
  case JV_KIND_NUMBER: {
    double d = jv_number_value(v);
    jv_free(v);
    return json_real(d);
  }
  case JV_KIND_STRING: {
    const char *str = jv_string_value(v);
    size_t len = strlen(str);
    if (!arena || !alloc_func) {
      // Use jansson's current allocator (may be arena or malloc)
      json_t *result = json_string(str);
      jv_free(v);
      return result;
    }
    char *arena_str = alloc_func(arena, len + 1);
    if (!arena_str) {
      // Fall back to regular jansson allocation
      json_t *result = json_string(str);
      jv_free(v);
      return result;
    }
    memcpy(arena_str, str, len);
    arena_str[len] = '\0';
    
    json_t *result = json_string(arena_str);  // Uses middleware's arena allocator
    jv_free(v);
    return result;
  }
  case JV_KIND_ARRAY: {
    json_t *arr = json_array();  // Uses main program's arena allocator
    jv_array_foreach(v, i, el) { 
      json_t *json_el = jv_to_json_with_arena(el, arena, alloc_func);
      if (json_el) {
        json_array_append_new(arr, json_el);
      }
    }
    jv_free(v);
    return arr;
  }
  case JV_KIND_OBJECT: {
    json_t *obj = json_object();  // Should use main program's arena allocator
    jv_object_foreach(v, k, val) {
      const char *key_str = jv_string_value(k);
      size_t key_len = strlen(key_str);
      if (!arena || !alloc_func) {
        // Use jansson's current allocator for key
        json_t *json_val = jv_to_json_with_arena(val, arena, alloc_func);
        if (json_val) {
          json_object_set_new(obj, key_str, json_val);
        }
        jv_free(k);
        continue;
      }
      char *arena_key = alloc_func(arena, key_len + 1);
      if (!arena_key) {
        // Fall back to regular jansson allocation
        json_t *json_val = jv_to_json_with_arena(val, arena, alloc_func);
        if (json_val) {
          json_object_set_new(obj, key_str, json_val);
        }
        jv_free(k);
        continue;
      }
      memcpy(arena_key, key_str, key_len);
      arena_key[key_len] = '\0';
      
      json_t *json_val = jv_to_json_with_arena(val, arena, alloc_func);
      if (json_val) {
        json_object_set_new(obj, arena_key, json_val);
      }
      jv_free(k);
    }
    jv_free(v);
    return obj;
  }
  }
  return NULL;
}

// Thread-local middleware arena context for internal use
static __thread void *current_middleware_arena = NULL;
static __thread arena_alloc_func current_middleware_alloc_func = NULL;

// The actual middleware function
json_t *middleware_execute(json_t *input, void *arena, void *alloc, void *free_func,
                       const char *filter) {
  // Set up thread-local arena context for string allocations
  current_middleware_arena = arena;
  current_middleware_alloc_func = (arena_alloc_func)alloc;
  
  // Use arena for string allocation in jv_to_json_with_arena
  arena_alloc_func alloc_func = (arena_alloc_func)alloc;
  (void)free_func; // Not used - arena is freed all at once
  
  // Handle null or empty filter
  if (!filter || strlen(filter) == 0) {
    filter = ".";  // Default to identity filter
  }
  
  jq_state *jq = get_cached_jq(filter);
  if (!jq) {
    json_t *error = json_object();
    json_object_set_new(error, "error",
                        json_string("Failed to compile JQ filter"));
    return error;
  }

  // Find the cache entry to get its mutex
  uint32_t hash = hash_string(filter) & HASH_MASK;
  jq_cache_entry *entry = jq_cache[hash];
  while (entry && strcmp(entry->filter, filter) != 0) {
    entry = entry->next;
  }

  if (!entry) {
    json_t *error = json_object();
    json_object_set_new(error, "error", json_string("Cache entry disappeared"));
    return error;
  }

  // Lock the entry for exclusive use
  pthread_mutex_lock(&entry->mutex);

  jv in = json_to_jv(input);
  jq_start(jq, in, 0);

  jv out = jq_next(jq);

  // Drain any additional results
  jv extra;
  while (jv_is_valid(extra = jq_next(jq))) {
    jv_free(extra);
  }

  // Convert jv to json while still holding the lock to prevent race conditions
  json_t *result = NULL;
  if (jv_is_valid(out)) {
    result = jv_to_json_with_arena(out, arena, alloc_func);
  }

  pthread_mutex_unlock(&entry->mutex);

  if (!result) {
    json_t *error = json_object();
    json_object_set_new(error, "error", json_string("JQ execution failed"));
    return error;
  }

  // Clear thread-local arena references to prevent use-after-free
  current_middleware_arena = NULL;
  current_middleware_alloc_func = NULL;
  
  return result;
}

