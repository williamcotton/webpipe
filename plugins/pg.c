#include <jansson.h>
#include <libpq-fe.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>

// Arena allocation function types for plugins
typedef void *(*arena_alloc_func)(void *arena, size_t size);
typedef void (*arena_free_func)(void *arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Global PostgreSQL connection with mutex protection
static PGconn *pg_connection = NULL;
static pthread_mutex_t pg_mutex = PTHREAD_MUTEX_INITIALIZER;
static int connection_failed = 0;

// Connection parameters (should be configurable)
static const char *pg_host = "localhost";
static const char *pg_port = "5432";
static const char *pg_dbname = "express-test";
static const char *pg_user = "postgres";
static const char *pg_password = "postgres";

// Function prototypes
static int pg_plugin_init(void);
static void pg_plugin_cleanup(void);
static json_t *pg_result_to_json(PGresult *result);
static json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func);
json_t *plugin_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *sql);
static void plugin_destructor(void);

// Initialize PostgreSQL connection (thread-safe)
int pg_plugin_init() {
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
  snprintf(conninfo, sizeof(conninfo),
           "host=%s port=%s dbname=%s user=%s password=%s", pg_host, pg_port,
           pg_dbname, pg_user, pg_password);

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
void pg_plugin_cleanup() {
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
    json_t *error_obj = json_object();
    json_object_set_new(error_obj, "error",
                        json_string(PQresultErrorMessage(result)));
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

// Execute SQL query with parameters
json_t *execute_sql(const char *sql, json_t *params, void *arena,
                    arena_alloc_func alloc_func) {
  // Try to initialize connection if needed
  if (!pg_plugin_init()) {
    json_t *error = json_object();
    json_object_set_new(error, "error",
                        json_string("Failed to connect to database"));
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
        // Convert integer to string using arena
        char *str = alloc_func(arena, 32);
        snprintf(str, 32, "%lld", json_integer_value(param));
        param_values[i] = str;
      } else if (json_is_real(param)) {
        // Convert real to string using arena
        char *str = alloc_func(arena, 32);
        snprintf(str, 32, "%f", json_real_value(param));
        param_values[i] = str;
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

// Plugin execute function
json_t *plugin_execute(json_t *input, void *arena, arena_alloc_func alloc_func,
                       arena_free_func free_func, const char *sql_query) {
  (void)free_func; // Not used - we don't free the arena

  // Look for sqlParams in input
  json_t *sql_params = json_object_get(input, "sqlParams");

  // Execute query
  json_t *result = execute_sql(sql_query, sql_params, arena, alloc_func);

  // Check if there was an error
  json_t *error = json_object_get(result, "error");
  if (error) {
    return result;
  }

  // Create response by copying input and adding data
  json_t *response = json_deep_copy(input);
  json_object_set_new(response, "data", result);

  return response;
}

// Plugin cleanup function called when plugin is unloaded
__attribute__((destructor)) void plugin_destructor() { pg_plugin_cleanup(); }

