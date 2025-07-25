#include <jansson.h>
#include <libpq-fe.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>

// Arena allocation function types for middlewares
typedef void *(*arena_alloc_func)(void *arena, size_t size);
typedef void (*arena_free_func)(void *arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Connection pool constants
#define INITIAL_POOL_SIZE 2
#define MAX_POOL_SIZE 10

// Connection pool structures
typedef struct PooledConnection {
    PGconn *conn;
    int in_use;
    struct PooledConnection *next;
} PooledConnection;

typedef struct ConnectionPool {
    MemoryArena *arena;
    char *conninfo;
    PooledConnection *connections;
    int size;
    int max_size;
    pthread_mutex_t lock;
} ConnectionPool;

// Function prototype for database registry
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func);

// Global PostgreSQL connection pool
static ConnectionPool *pg_pool = NULL;
static int connection_failed = 0;

// Connection parameters from configuration block
static const char *get_pg_string_config(json_t *config, const char *key) {
  if (!config) return NULL;
  
  json_t *value = json_object_get(config, key);
  if (value && json_is_string(value)) {
    return json_string_value(value);
  }
  
  return NULL;
}


static bool get_pg_bool_config(json_t *config, const char *key, bool default_value) {
  if (!config) return default_value;
  
  json_t *value = json_object_get(config, key);
  if (value && json_is_boolean(value)) {
    return json_boolean_value(value);
  }
  
  return default_value;
}

static int get_pg_int_config(json_t *config, const char *key, int default_value) {
  if (!config) return default_value;
  
  json_t *value = json_object_get(config, key);
  if (value && json_is_integer(value)) {
    return (int)json_integer_value(value);
  }
  
  return default_value;
}

// Function prototypes
static int pg_middleware_init(json_t *config);
static void pg_middleware_cleanup(void);
static json_t *pg_result_to_json(PGresult *result);
static json_t *execute_sql_internal(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func, json_t *config);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *sql, json_t *middleware_config, char **contentType, json_t *variables);
int middleware_init(json_t *config);
static void middleware_destructor(void);

// Pool management function prototypes
static PooledConnection* create_connection(ConnectionPool *pool, json_t *config);
static ConnectionPool* init_connection_pool(MemoryArena *arena, json_t *config);
static PooledConnection* get_connection(ConnectionPool *pool);
static void return_connection(ConnectionPool *pool, PooledConnection *conn);
static void close_connection_pool(ConnectionPool *pool);

// Global variable to store current middleware config for database registry
static json_t *current_middleware_config = NULL;

// Create a single connection for the pool
static PooledConnection* create_connection(ConnectionPool *pool, json_t *config) {
    if (!pool || !config) return NULL;
    
    // We need to use a global arena allocator for pool structures
    // For now, we'll use malloc for pool structures and manage cleanup manually
    PooledConnection *conn = malloc(sizeof(PooledConnection));
    if (!conn) return NULL;
    
    // Build connection string
    char conninfo[512];
    const char *host = get_pg_string_config(config, "host");
    const char *port = get_pg_string_config(config, "port");
    const char *dbname = get_pg_string_config(config, "database");
    const char *user = get_pg_string_config(config, "user");
    const char *password = get_pg_string_config(config, "password");
    bool ssl = get_pg_bool_config(config, "ssl", false);
    
    if (!host || !port || !dbname || !user || !password) {
        free(conn);
        return NULL;
    }
    
    if (ssl) {
        snprintf(conninfo, sizeof(conninfo),
                 "host=%s port=%s dbname=%s user=%s password=%s sslmode=require gssencmode=disable", 
                 host, port, dbname, user, password);
    } else {
        snprintf(conninfo, sizeof(conninfo),
                 "host=%s port=%s dbname=%s user=%s password=%s sslmode=disable gssencmode=disable", 
                 host, port, dbname, user, password);
    }
    
    conn->conn = PQconnectdb(conninfo);
    if (PQstatus(conn->conn) != CONNECTION_OK) {
        fprintf(stderr, "Failed to create database connection: %s\n", 
                PQerrorMessage(conn->conn));
        PQfinish(conn->conn);
        free(conn);
        return NULL;
    }
    
    conn->in_use = 0;
    conn->next = NULL;
    return conn;
}

// Initialize connection pool
static ConnectionPool* init_connection_pool(MemoryArena *arena, json_t *config) {
    if (!config) return NULL;
    
    ConnectionPool *pool = malloc(sizeof(ConnectionPool));
    if (!pool) return NULL;
    
    pool->arena = arena;
    pool->connections = NULL;
    pool->size = 0;
    
    // Get pool size configuration
    int initial_size = get_pg_int_config(config, "initialPoolSize", INITIAL_POOL_SIZE);
    int max_size = get_pg_int_config(config, "maxPoolSize", MAX_POOL_SIZE);
    
    // Validate pool sizes
    if (initial_size < 1) initial_size = 1;
    if (max_size < initial_size) max_size = initial_size;
    if (max_size > 50) max_size = 50; // Reasonable upper limit
    
    pool->max_size = max_size;
    
    if (pthread_mutex_init(&pool->lock, NULL) != 0) {
        fprintf(stderr, "Failed to initialize pool mutex\n");
        return NULL;
    }
    
    // Create initial connections
    for (int i = 0; i < initial_size; i++) {
        PooledConnection *conn = create_connection(pool, config);
        if (!conn) {
            fprintf(stderr, "Warning: Could only create %d initial connections\n", i);
            break;
        }
        
        conn->next = pool->connections;
        pool->connections = conn;
        pool->size++;
    }
    
    if (pool->size == 0) {
        fprintf(stderr, "Failed to create any initial connections\n");
        pthread_mutex_destroy(&pool->lock);
        return NULL;
    }
    
    return pool;
}

// Get available connection from pool
static PooledConnection* get_connection(ConnectionPool *pool) {
    if (!pool) return NULL;
    
    pthread_mutex_lock(&pool->lock);
    
    // Look for an available connection
    PooledConnection *conn = pool->connections;
    while (conn) {
        if (!conn->in_use) {
            conn->in_use = 1;
            pthread_mutex_unlock(&pool->lock);
            return conn;
        }
        conn = conn->next;
    }
    
    // If we have room, create a new connection
    if (pool->size < pool->max_size) {
        conn = create_connection(pool, current_middleware_config);
        if (conn) {
            conn->in_use = 1;
            conn->next = pool->connections;
            pool->connections = conn;
            pool->size++;
            pthread_mutex_unlock(&pool->lock);
            return conn;
        }
    }
    
    pthread_mutex_unlock(&pool->lock);
    return NULL;  // No connections available
}

// Return connection to pool
static void return_connection(ConnectionPool *pool, PooledConnection *conn) {
    if (!pool || !conn) return;
    
    pthread_mutex_lock(&pool->lock);
    
    // Check connection status and reset if needed
    if (PQstatus(conn->conn) != CONNECTION_OK) {
        PQreset(conn->conn);
    }
    
    conn->in_use = 0;
    
    pthread_mutex_unlock(&pool->lock);
}

// Close connection pool
static void close_connection_pool(ConnectionPool *pool) {
    if (!pool) return;
    
    pthread_mutex_lock(&pool->lock);
    
    PooledConnection *conn = pool->connections;
    while (conn) {
        PooledConnection *next = conn->next;
        PQfinish(conn->conn);
        free(conn);  // Free individual connections
        conn = next;
    }
    
    pthread_mutex_unlock(&pool->lock);
    pthread_mutex_destroy(&pool->lock);
    free(pool);  // Free the pool structure itself
}

// Initialize PostgreSQL connection (thread-safe)
int pg_middleware_init(json_t *config) {
  // If we already have a pool or already failed, return early
  if (pg_pool) {
    return 1; // Success
  }

  if (connection_failed) {
    return 0; // Already failed
  }

  // Validate required configuration
  const char *host = get_pg_string_config(config, "host");
  const char *port = get_pg_string_config(config, "port");
  const char *dbname = get_pg_string_config(config, "database");
  const char *user = get_pg_string_config(config, "user");
  const char *password = get_pg_string_config(config, "password");
  
  if (!host || !port || !dbname || !user || !password) {
    fprintf(stderr, "pg: Missing required configuration. Need host, port, database, user, and password\n");
    connection_failed = 1;
    return 0;
  }

  // Initialize connection pool (using NULL for arena since we're using malloc)
  pg_pool = init_connection_pool(NULL, config);
  if (!pg_pool) {
    fprintf(stderr, "pg: Failed to initialize connection pool\n");
    connection_failed = 1;
    return 0; // Failed
  }

  printf("pg: Connection pool initialized with %d connections (max: %d)\n", 
         pg_pool->size, pg_pool->max_size);
  return 1; // Success
}

// Cleanup PostgreSQL connection pool
void pg_middleware_cleanup() {
  if (pg_pool) {
    close_connection_pool(pg_pool);
    pg_pool = NULL;
  }
  connection_failed = 0;
}

// Convert PostgreSQL result to JSON
json_t *pg_result_to_json(PGresult *result) {
  if (!result) {
    return json_null();
  }

  ExecStatusType status = PQresultStatus(result);

  if (status != PGRES_TUPLES_OK && status != PGRES_COMMAND_OK) {
    // Return standardized error format
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string("sqlError"));
    json_object_set_new(error_detail, "message", json_string(PQresultErrorMessage(result)));
    
    // Add SQL state if available
    char *sqlstate = PQresultErrorField(result, PG_DIAG_SQLSTATE);
    if (sqlstate) {
      json_object_set_new(error_detail, "sqlstate", json_string(sqlstate));
    }
    
    // Add severity if available
    char *severity = PQresultErrorField(result, PG_DIAG_SEVERITY);
    if (severity) {
      json_object_set_new(error_detail, "severity", json_string(severity));
    }
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
  }

  json_t *response = json_object();

  if (status == PGRES_TUPLES_OK) {
    int rows = PQntuples(result);
    int cols = PQnfields(result);

    json_t *data_array = json_array();

    for (int row = 0; row < rows; row++) {
      json_t *row_obj = json_object();

      for (int col = 0; col < cols; col++) {
        const char *field_name = PQfname(result, col);
        const char *field_value = PQgetvalue(result, row, col);

        if (PQgetisnull(result, row, col)) {
          json_object_set_new(row_obj, field_name, json_null());
        } else {
          // Try to determine type based on PostgreSQL type OID
          Oid field_type = PQftype(result, col);

          switch (field_type) {
          case 16: // bool
            json_object_set_new(row_obj, field_name,
                                json_boolean(field_value[0] == 't'));
            break;
          case 20: // int8
          case 21: // int2
          case 23: // int4
            json_object_set_new(row_obj, field_name,
                                json_integer(atoll(field_value)));
            break;
          case 700:  // float4
          case 701:  // float8
          case 1700: // numeric
            json_object_set_new(row_obj, field_name,
                                json_real(atof(field_value)));
            break;
          default:
            json_object_set_new(row_obj, field_name, json_string(field_value));
            break;
          }
        }
      }

      json_array_append_new(data_array, row_obj);
    }

    json_object_set_new(response, "rows", data_array);
    json_object_set_new(response, "rowCount", json_integer(rows));
  } else {
    // Command completed successfully
    char *affected = PQcmdTuples(result);
    json_object_set_new(response, "rowCount", json_integer(atoi(affected)));
  }

  return response;
}

// Execute SQL query with parameters (internal)
static json_t *execute_sql_internal(const char *sql, json_t *params, void *arena,
                                   arena_alloc_func alloc_func, json_t *config) {
  // Try to initialize connection pool if needed
  if (!pg_middleware_init(config)) {
    json_t *error = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string("sqlError"));
    json_object_set_new(error_detail, "message", json_string("Failed to connect to database"));
    json_object_set_new(error_detail, "sqlstate", json_string("08000")); // Connection exception
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error, "errors", errors_array);
    
    return error;
  }

  // Get connection from pool
  PooledConnection *pooled_conn = get_connection(pg_pool);
  if (!pooled_conn) {
    json_t *error = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string("sqlError"));
    json_object_set_new(error_detail, "message", json_string("No available database connections"));
    json_object_set_new(error_detail, "sqlstate", json_string("08004")); // Connection rejected
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error, "errors", errors_array);
    
    return error;
  }

  PGresult *result = NULL;
  PGconn *conn = pooled_conn->conn;

  if (params && json_is_array(params)) {
    // Parameterized query
    size_t param_count = json_array_size(params);
    const char **param_values = alloc_func(arena, sizeof(char *) * param_count);

    for (size_t i = 0; i < param_count; i++) {
      json_t *param = json_array_get(params, i);

      if (json_is_string(param)) {
        param_values[i] = json_string_value(param);
      } else if (json_is_integer(param)) {
        // Convert integer to string using arena (no decimal point)
        char *str = alloc_func(arena, 32);
        snprintf(str, 32, "%ld", (long)json_integer_value(param));
        param_values[i] = str;
      } else if (json_is_real(param)) {
        // Convert real to string using arena
        char *str = alloc_func(arena, 32);
        snprintf(str, 32, "%f", json_real_value(param));
        param_values[i] = str;
      } else if (json_is_boolean(param)) {
        // Convert boolean to string using arena
        param_values[i] = json_boolean_value(param) ? "true" : "false";
      } else if (json_is_null(param)) {
        param_values[i] = NULL;
      } else {
        param_values[i] = "NULL";
      }
    }

    result = PQexecParams(conn, sql, (int)param_count, NULL, param_values,
                          NULL, NULL, 0);
  } else {
    // Simple query
    result = PQexec(conn, sql);
  }

  // Return connection to pool
  return_connection(pg_pool, pooled_conn);

  json_t *response = pg_result_to_json(result);
  PQclear(result);

  return response;
}

// Public execute_sql function for database registry
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func) {
  return execute_sql_internal(sql, params, arena, alloc_func, current_middleware_config);
}

// Public middleware initialization function called by server at startup
int middleware_init(json_t *config) {
  // Store the configuration for later use by execute_sql
  current_middleware_config = config;
  
  // Initialize the PostgreSQL connection
  // pg_middleware_init returns 1 for success, 0 for failure
  // middleware_init should return 0 for success, non-zero for failure
  return pg_middleware_init(config) ? 0 : 1;
}

// Middleware execute function
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func,
                       arena_free_func free_func, const char *sql_query, json_t *middleware_config, char **contentType, json_t *variables) {
  (void)free_func; // Not used - we don't free the arena
  (void)contentType; // PostgreSQL middleware produces JSON output, so we don't change content type
  (void)variables; // Unused parameter

  // Look for sqlParams in input
  json_t *sql_params = json_object_get(input, "sqlParams");

  // Store current middleware config for database registry
  current_middleware_config = middleware_config;
  
  // Execute query using middleware configuration
  json_t *result = execute_sql_internal(sql_query, sql_params, arena, alloc_func, middleware_config);

  // Check if there was an error (using standardized format)
  json_t *errors = json_object_get(result, "errors");
  if (errors) {
    // Add the SQL query to the error for debugging
    json_t *first_error = json_array_get(errors, 0);
    if (first_error) {
      json_object_set_new(first_error, "query", json_string(sql_query));
    }
    
    // Copy input and add the errors
    json_t *response = json_deep_copy(input);
    json_object_set_new(response, "errors", json_deep_copy(errors));
    
    return response;
  }

  // Create response by copying input
  json_t *response = json_deep_copy(input);
  
  // Check for resultName field to determine how to store the result
  json_t *result_name = json_object_get(input, "resultName");
  
  if (result_name && json_is_string(result_name)) {
    // Named result: store under data.resultName
    const char *name = json_string_value(result_name);
    
    // Get or create data object
    json_t *data_obj = json_object_get(response, "data");
    if (!data_obj || !json_is_object(data_obj)) {
      data_obj = json_object();
      json_object_set_new(response, "data", data_obj);
    }
    
    // Store result under the named key
    json_object_set_new(data_obj, name, result);
  } else {
    // Legacy behavior: store result directly in data
    json_object_set_new(response, "data", result);
  }

  return response;
}

// Middleware cleanup function called when middleware is unloaded
__attribute__((destructor)) void middleware_destructor() { pg_middleware_cleanup(); }

