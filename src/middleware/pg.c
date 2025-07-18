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

// Function prototype for database registry
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func);

// Global PostgreSQL connection with mutex protection
static PGconn *pg_connection = NULL;
static pthread_mutex_t pg_mutex = PTHREAD_MUTEX_INITIALIZER;
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

// Function prototypes
static int pg_middleware_init(json_t *config);
static void pg_middleware_cleanup(void);
static json_t *pg_result_to_json(PGresult *result);
static json_t *execute_sql_internal(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func, json_t *config);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *sql, json_t *middleware_config, char **contentType, json_t *variables);
int middleware_init(json_t *config);
static void middleware_destructor(void);

// Global variable to store current middleware config for database registry
static json_t *current_middleware_config = NULL;

// Initialize PostgreSQL connection (thread-safe)
int pg_middleware_init(json_t *config) {
  pthread_mutex_lock(&pg_mutex);

  // If we already have a connection or already failed, return early
  if (pg_connection) {
    pthread_mutex_unlock(&pg_mutex);
    return 1; // Success
  }

  if (connection_failed) {
    pthread_mutex_unlock(&pg_mutex);
    return 0; // Already failed
  }

  // Try to connect
  char conninfo[512];
  
  // Get configuration values from config block
  const char *host = get_pg_string_config(config, "host");
  const char *port = get_pg_string_config(config, "port");
  const char *dbname = get_pg_string_config(config, "database");
  const char *user = get_pg_string_config(config, "user");
  const char *password = get_pg_string_config(config, "password");
  
  // Validate required configuration
  if (!host || !port || !dbname || !user || !password) {
    fprintf(stderr, "pg: Missing required configuration. Need host, port, database, user, and password\n");
    connection_failed = 1;
    pthread_mutex_unlock(&pg_mutex);
    return 0;
  }
  
  // Get optional SSL configuration
  bool ssl = get_pg_bool_config(config, "ssl", false);
  
  // Estimate required buffer size (with some margin for format string)
  size_t required_size = strlen(host) + strlen(port) + strlen(dbname) + 
                        strlen(user) + strlen(password) + 150;  // 150 for format string + SSL
  
  if (required_size >= sizeof(conninfo)) {
    fprintf(stderr, "Error: PostgreSQL connection string too long (%zu bytes, max %zu)\n", 
            required_size, sizeof(conninfo));
    return 0;
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

  PGconn *new_conn = PQconnectdb(conninfo);

  if (PQstatus(new_conn) != CONNECTION_OK) {
    // Get error message before calling PQfinish
    char error_msg[256];
    snprintf(error_msg, sizeof(error_msg), "pg: Connection failed: %s",
             PQerrorMessage(new_conn));
    fprintf(stderr, "%s\n", error_msg);

    PQfinish(new_conn);
    connection_failed = 1;
    pthread_mutex_unlock(&pg_mutex);
    return 0; // Failed
  }

  pg_connection = new_conn;
  printf("pg: Connected to PostgreSQL database\n");
  pthread_mutex_unlock(&pg_mutex);
  return 1; // Success
}

// Cleanup PostgreSQL connection
void pg_middleware_cleanup() {
  pthread_mutex_lock(&pg_mutex);
  if (pg_connection) {
    PQfinish(pg_connection);
    pg_connection = NULL;
  }
  connection_failed = 0;
  pthread_mutex_unlock(&pg_mutex);
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
  // Try to initialize connection if needed
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

  PGresult *result = NULL;

  // Lock for the duration of the query
  pthread_mutex_lock(&pg_mutex);

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

    result = PQexecParams(pg_connection, sql, (int)param_count, NULL, param_values,
                          NULL, NULL, 0);
  } else {
    // Simple query
    result = PQexec(pg_connection, sql);
  }

  pthread_mutex_unlock(&pg_mutex);

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

  // Create response by copying input and adding data
  json_t *response = json_deep_copy(input);
  json_object_set_new(response, "data", result);

  return response;
}

// Middleware cleanup function called when middleware is unloaded
__attribute__((destructor)) void middleware_destructor() { pg_middleware_cleanup(); }

