#include <microhttpd.h>
#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>
#include <sys/stat.h>
#include <stdbool.h>
#include <pthread.h>

// Include the existing wp.c definitions
#include "wp.h"

// Include lexer/parser implementation
#include <ctype.h>

// Helper functions
char *strdup_safe(const char *s) {
  if (!s)
    return NULL;
  size_t len = strlen(s);
  char *copy = malloc(len + 1);
  if (copy) {
    strcpy(copy, s);
  }
  return copy;
}

// Lexer functions
Lexer *lexer_new(const char *source) {
  Lexer *lexer = malloc(sizeof(Lexer));
  lexer->source = source;
  lexer->current = 0;
  lexer->line = 1;
  lexer->column = 1;
  return lexer;
}

void lexer_free(Lexer *lexer) { free(lexer); }

char lexer_peek(Lexer *lexer) {
  if ((size_t)lexer->current >= strlen(lexer->source))
    return '\0';
  return lexer->source[lexer->current];
}

char lexer_advance(Lexer *lexer) {
  char c = lexer_peek(lexer);
  if (c != '\0') {
    lexer->current++;
    if (c == '\n') {
      lexer->line++;
      lexer->column = 1;
    } else {
      lexer->column++;
    }
  }
  return c;
}

void lexer_skip_whitespace(Lexer *lexer) {
  while (isspace(lexer_peek(lexer)) && lexer_peek(lexer) != '\n') {
    lexer_advance(lexer);
  }
}

Token lexer_make_token(Lexer *lexer, TokenType type, const char *value) {
  Token token;
  token.type = type;
  token.value = strdup_safe(value);
  token.line = lexer->line;
  token.column = lexer->column;
  return token;
}

Token lexer_read_string(Lexer *lexer) {
  lexer_advance(lexer); // Skip opening backtick
  int start = lexer->current;

  while (lexer_peek(lexer) != '`' && lexer_peek(lexer) != '\0') {
    lexer_advance(lexer);
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  lexer_advance(lexer); // Skip closing backtick

  Token token = lexer_make_token(lexer, TOKEN_STRING, value);
  free(value);
  return token;
}

Token lexer_read_identifier(Lexer *lexer) {
  int start = lexer->current;

  while (isalnum(lexer_peek(lexer)) || lexer_peek(lexer) == '_') {
    lexer_advance(lexer);
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  TokenType type = TOKEN_IDENTIFIER;
  if (strcmp(value, "GET") == 0 || strcmp(value, "POST") == 0 ||
      strcmp(value, "PUT") == 0 || strcmp(value, "DELETE") == 0) {
    type = TOKEN_HTTP_METHOD;
  }

  Token token = lexer_make_token(lexer, type, value);
  free(value);
  return token;
}

Token lexer_read_route(Lexer *lexer) {
  int start = lexer->current;

  while (lexer_peek(lexer) != '\0' && lexer_peek(lexer) != '\n' &&
         lexer_peek(lexer) != ' ' && lexer_peek(lexer) != '\t') {
    char c = lexer_peek(lexer);
    if (isalnum(c) || c == '/' || c == ':' || c == '-' || c == '_') {
      lexer_advance(lexer);
    } else {
      break;
    }
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  Token token = lexer_make_token(lexer, TOKEN_ROUTE, value);
  free(value);
  return token;
}

Token lexer_read_number(Lexer *lexer) {
  int start = lexer->current;

  while (isdigit(lexer_peek(lexer))) {
    lexer_advance(lexer);
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  Token token = lexer_make_token(lexer, TOKEN_NUMBER, value);
  free(value);
  return token;
}

Token lexer_next_token(Lexer *lexer) {
  lexer_skip_whitespace(lexer);

  char c = lexer_peek(lexer);

  if (c == '\0') {
    return lexer_make_token(lexer, TOKEN_EOF, "");
  }

  if (c == '\n') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_NEWLINE, "\n");
  }

  if (c == '|' && lexer->source[lexer->current + 1] == '>') {
    lexer_advance(lexer);
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_PIPE, "|>");
  }

  if (c == ':') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_COLON, ":");
  }

  if (c == '=') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_EQUALS, "=");
  }

  if (c == '{') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_LBRACE, "{");
  }

  if (c == '}') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_RBRACE, "}");
  }

  if (c == '(') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_LPAREN, "(");
  }

  if (c == ')') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_RPAREN, ")");
  }

  if (c == '`') {
    return lexer_read_string(lexer);
  }

  if (c == '/') {
    return lexer_read_route(lexer);
  }

  if (isdigit(c)) {
    return lexer_read_number(lexer);
  }

  if (isalpha(c) || c == '_') {
    return lexer_read_identifier(lexer);
  }

  // Skip unknown character
  lexer_advance(lexer);
  return lexer_next_token(lexer);
}

Token *lexer_tokenize(const char *source, int *token_count) {
  Lexer *lexer = lexer_new(source);
  Token *tokens = malloc(sizeof(Token) * 1000); // Max 1000 tokens
  int count = 0;

  Token token;
  do {
    token = lexer_next_token(lexer);
    tokens[count++] = token;
  } while (token.type != TOKEN_EOF);

  *token_count = count;
  lexer_free(lexer);
  return tokens;
}

// Parser functions
Parser *parser_new(Token *tokens, int token_count) {
  Parser *parser = malloc(sizeof(Parser));
  parser->tokens = tokens;
  parser->token_count = token_count;
  parser->current = 0;
  return parser;
}

void parser_free(Parser *parser) { free(parser); }

bool parser_is_at_end(Parser *parser) {
  return parser->current >= parser->token_count ||
         parser->tokens[parser->current].type == TOKEN_EOF;
}

Token *parser_peek(Parser *parser) {
  if (parser_is_at_end(parser))
    return NULL;
  return &parser->tokens[parser->current];
}

Token *parser_advance(Parser *parser) {
  if (!parser_is_at_end(parser))
    parser->current++;
  return &parser->tokens[parser->current - 1];
}

bool parser_check(Parser *parser, TokenType type) {
  if (parser_is_at_end(parser))
    return false;
  return parser_peek(parser)->type == type;
}

bool parser_match(Parser *parser, TokenType type) {
  if (parser_check(parser, type)) {
    parser_advance(parser);
    return true;
  }
  return false;
}

void parser_consume_newlines(Parser *parser) {
  while (parser_match(parser, TOKEN_NEWLINE)) {
  }
}

PipelineStep *parser_parse_pipeline(Parser *parser) {
  PipelineStep *head = NULL;
  PipelineStep *tail = NULL;

  while (parser_match(parser, TOKEN_PIPE)) {
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected plugin name after |>\n");
      return head;
    }

    Token *plugin = parser_advance(parser);

    // Check for result step
    if (strcmp(plugin->value, "result") == 0) {
      // This is a result step - parse result conditions
      if (!parser_match(parser, TOKEN_LBRACE)) {
        fprintf(stderr, "Expected { after result\n");
        return head;
      }

      ASTNode *result_node = parser_parse_result_step(parser);
      
      // Create a special pipeline step for result
      PipelineStep *step = malloc(sizeof(PipelineStep));
      step->plugin = strdup_safe("result");
      step->value = (char*)result_node; // Store result node as value
      step->is_variable = false;
      step->next = NULL;

      if (tail) {
        tail->next = step;
      } else {
        head = step;
      }
      tail = step;

      parser_consume_newlines(parser);
      continue;
    }

    if (!parser_match(parser, TOKEN_COLON)) {
      fprintf(stderr, "Expected : after plugin name\n");
      return head;
    }

    char *value = NULL;
    bool is_variable = false;

    if (parser_check(parser, TOKEN_STRING)) {
      value = strdup_safe(parser_advance(parser)->value);
      is_variable = false;
    } else if (parser_check(parser, TOKEN_IDENTIFIER)) {
      value = strdup_safe(parser_advance(parser)->value);
      is_variable = true;
    }

    PipelineStep *step = malloc(sizeof(PipelineStep));
    step->plugin = strdup_safe(plugin->value);
    step->value = value;
    step->is_variable = is_variable;
    step->next = NULL;

    if (tail) {
      tail->next = step;
    } else {
      head = step;
    }
    tail = step;

    parser_consume_newlines(parser);
  }

  return head;
}

ASTNode *parser_parse_result_step(Parser *parser) {
  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_RESULT_STEP;
  node->data.result_step.conditions = NULL;

  ResultCondition *head = NULL;
  ResultCondition *tail = NULL;

  while (!parser_check(parser, TOKEN_RBRACE) && !parser_is_at_end(parser)) {
    // Parse condition: name(status_code) { pipeline }
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected condition name\n");
      break;
    }

    Token *condition_name = parser_advance(parser);

    if (!parser_match(parser, TOKEN_LPAREN)) {
      fprintf(stderr, "Expected ( after condition name\n");
      break;
    }

    if (!parser_check(parser, TOKEN_NUMBER)) {
      fprintf(stderr, "Expected status code number\n");
      break;
    }

    Token *status_code = parser_advance(parser);

    if (!parser_match(parser, TOKEN_RPAREN)) {
      fprintf(stderr, "Expected ) after status code\n");
      break;
    }

    if (!parser_match(parser, TOKEN_LBRACE)) {
      fprintf(stderr, "Expected { after condition\n");
      break;
    }

    parser_consume_newlines(parser);

    // Parse pipeline for this condition
    PipelineStep *pipeline = parser_parse_pipeline(parser);

    if (!parser_match(parser, TOKEN_RBRACE)) {
      fprintf(stderr, "Expected } after condition pipeline\n");
      break;
    }

    ResultCondition *condition = malloc(sizeof(ResultCondition));
    condition->condition_name = strdup_safe(condition_name->value);
    condition->status_code = atoi(status_code->value);
    condition->pipeline = pipeline;
    condition->next = NULL;

    if (tail) {
      tail->next = condition;
    } else {
      head = condition;
    }
    tail = condition;

    parser_consume_newlines(parser);
  }

  node->data.result_step.conditions = head;

  if (!parser_match(parser, TOKEN_RBRACE)) {
    fprintf(stderr, "Expected } at end of result step\n");
  }

  return node;
}

ASTNode *parser_parse_route_definition(Parser *parser) {
  Token *method = parser_advance(parser);

  if (!parser_check(parser, TOKEN_ROUTE)) {
    fprintf(stderr, "Expected route after HTTP method\n");
    return NULL;
  }

  Token *route = parser_advance(parser);
  parser_consume_newlines(parser);

  PipelineStep *pipeline = parser_parse_pipeline(parser);

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_ROUTE_DEFINITION;
  node->data.route_def.method = strdup_safe(method->value);
  node->data.route_def.route = strdup_safe(route->value);
  node->data.route_def.pipeline = pipeline;

  return node;
}

ASTNode *parser_parse_variable_assignment(Parser *parser) {
  Token *plugin = parser_advance(parser);
  Token *name = parser_advance(parser);

  if (!parser_match(parser, TOKEN_EQUALS)) {
    fprintf(stderr, "Expected = in variable assignment\n");
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected string value in variable assignment\n");
    return NULL;
  }

  Token *value = parser_advance(parser);

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_VARIABLE_ASSIGNMENT;
  node->data.var_assign.plugin = strdup_safe(plugin->value);
  node->data.var_assign.name = strdup_safe(name->value);
  node->data.var_assign.value = strdup_safe(value->value);

  return node;
}

ASTNode *parser_parse_statement(Parser *parser) {
  if (parser_check(parser, TOKEN_HTTP_METHOD)) {
    return parser_parse_route_definition(parser);
  }

  // Check for variable assignment
  if (parser_check(parser, TOKEN_IDENTIFIER)) {
    int saved = parser->current;
    parser_advance(parser); // plugin
    if (parser_check(parser, TOKEN_IDENTIFIER)) {
      parser_advance(parser); // name
      if (parser_check(parser, TOKEN_EQUALS)) {
        parser->current = saved;
        return parser_parse_variable_assignment(parser);
      }
    }
    parser->current = saved;
  }

  return NULL;
}

ASTNode *parser_parse(Parser *parser) {
  ASTNode *program = malloc(sizeof(ASTNode));
  program->type = AST_PROGRAM;
  program->data.program.statements = malloc(sizeof(ASTNode *) * 100);
  program->data.program.statement_count = 0;

  while (!parser_is_at_end(parser)) {
    if (parser_match(parser, TOKEN_NEWLINE))
      continue;

    ASTNode *stmt = parser_parse_statement(parser);
    if (stmt) {
      program->data.program
          .statements[program->data.program.statement_count++] = stmt;
    }
  }

  return program;
}

// Memory cleanup
void free_pipeline(PipelineStep *pipeline) {
  while (pipeline) {
    PipelineStep *next = pipeline->next;
    free(pipeline->plugin);
    free(pipeline->value);
    free(pipeline);
    pipeline = next;
  }
}

void free_result_conditions(ResultCondition *conditions) {
  while (conditions) {
    ResultCondition *next = conditions->next;
    free(conditions->condition_name);
    free_pipeline(conditions->pipeline);
    free(conditions);
    conditions = next;
  }
}

void free_ast(ASTNode *node) {
  if (!node)
    return;

  switch (node->type) {
  case AST_PROGRAM:
    for (int i = 0; i < node->data.program.statement_count; i++) {
      free_ast(node->data.program.statements[i]);
    }
    free(node->data.program.statements);
    break;

  case AST_ROUTE_DEFINITION:
    free(node->data.route_def.method);
    free(node->data.route_def.route);
    free_pipeline(node->data.route_def.pipeline);
    break;

  case AST_VARIABLE_ASSIGNMENT:
    free(node->data.var_assign.plugin);
    free(node->data.var_assign.name);
    free(node->data.var_assign.value);
    break;

  case AST_RESULT_STEP:
    free_result_conditions(node->data.result_step.conditions);
    break;
  }

  free(node);
}

void free_tokens(Token *tokens, int count) {
  for (int i = 0; i < count; i++) {
    free(tokens[i].value);
  }
  free(tokens);
}

// Memory arena functions - definition moved to wp.h

// Plugin interface
// Remove this block:
// typedef struct {
//     char *name;
//     void *handle;
//     json_t *(*execute)(json_t *input, MemoryArena *arena, const char *config);
// } Plugin;

// Runtime state
typedef struct {
    struct MHD_Daemon *daemon;
    ASTNode *program;
    Plugin *plugins;
    int plugin_count;
    json_t *variables;
} WPRuntime;

// Global runtime instance
static WPRuntime *runtime = NULL;

// Memory arena functions
MemoryArena *arena_create(size_t size) {
    MemoryArena *arena = malloc(sizeof(MemoryArena));
    arena->memory = malloc(size);
    arena->size = size;
    arena->used = 0;
    return arena;
}

void *arena_alloc(MemoryArena *arena, size_t size) {
    if (arena->used + size > arena->size) {
        return NULL; // Out of memory
    }
    void *ptr = arena->memory + arena->used;
    arena->used += size;
    return ptr;
}

void arena_free(MemoryArena *arena) {
    free(arena->memory);
    free(arena);
}

// Thread-local storage for current arena
static pthread_key_t current_arena_key;
static pthread_once_t arena_key_once = PTHREAD_ONCE_INIT;

static void arena_key_init(void) {
    pthread_key_create(&current_arena_key, NULL);
}

// Set current arena for this thread
void set_current_arena(MemoryArena *arena) {
    pthread_once(&arena_key_once, arena_key_init);
    pthread_setspecific(current_arena_key, arena);
}

// Get current arena for this thread
MemoryArena *get_current_arena(void) {
    pthread_once(&arena_key_once, arena_key_init);
    return pthread_getspecific(current_arena_key);
}

// Custom jansson allocator functions
static void *jansson_arena_malloc(size_t size) {
    MemoryArena *arena = get_current_arena();
    if (arena) {
        return arena_alloc(arena, size);
    }
    return malloc(size); // Fallback
}

static void jansson_arena_free(void *ptr) {
    // Arena memory is freed all at once, so we don't need to do anything here
    // But we can't use free() because the pointer might be from the arena
    // When arena is NULL, this means we're in cleanup phase and should ignore
    (void)ptr; // Suppress unused parameter warning
}

// Wrapper functions for plugin interface
static void *arena_alloc_wrapper(void *arena, size_t size) {
    return arena_alloc((MemoryArena*)arena, size);
}

static void arena_free_wrapper(void *arena) {
    arena_free((MemoryArena*)arena);
}

// Plugin loading and management
int load_plugin(const char *name) {
    char plugin_path[256];
    snprintf(plugin_path, sizeof(plugin_path), "./plugins/%s.so", name);
    
    void *handle = dlopen(plugin_path, RTLD_LAZY);
    if (!handle) {
        fprintf(stderr, "Error loading plugin %s: %s\n", name, dlerror());
        return -1;
    }
    
    // Get plugin execute function
    json_t *(*execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *) = dlsym(handle, "plugin_execute");
    if (!execute) {
        fprintf(stderr, "Error getting plugin_execute for %s: %s\n", name, dlerror());
        dlclose(handle);
        return -1;
    }
    
    // Add to runtime plugins
    runtime->plugins = realloc(runtime->plugins, sizeof(Plugin) * (runtime->plugin_count + 1));
    runtime->plugins[runtime->plugin_count].name = strdup(name);
    runtime->plugins[runtime->plugin_count].handle = handle;
    runtime->plugins[runtime->plugin_count].execute = execute;
    runtime->plugin_count++;
    
    return 0;
}

Plugin *find_plugin(const char *name) {
    for (int i = 0; i < runtime->plugin_count; i++) {
        if (strcmp(runtime->plugins[i].name, name) == 0) {
            return &runtime->plugins[i];
        }
    }
    return NULL;
}

// HTTP request handling
json_t *create_request_json(struct MHD_Connection *connection, 
                           const char *url, const char *method,
                           const char *upload_data, size_t *upload_data_size) {
    json_t *request = json_object();
    
    // Set method
    json_object_set_new(request, "method", json_string(method));
    
    // Set URL
    json_object_set_new(request, "url", json_string(url));
    
    // Parse URL params (simple implementation)
    json_t *params = json_object();
    json_object_set_new(request, "params", params);
    
    // Parse query string
    json_t *query = json_object();
    json_object_set_new(request, "query", query);
    
    // Set body if present
    if (upload_data && *upload_data_size > 0) {
        json_object_set_new(request, "body", json_string(upload_data));
    } else {
        json_object_set_new(request, "body", json_null());
    }
    
    // Headers
    json_t *headers = json_object();
    json_object_set_new(request, "headers", headers);
    
    return request;
}

int execute_pipeline_with_result(PipelineStep *pipeline, json_t *request, MemoryArena *arena, 
                                json_t **final_response, int *response_code) {
    json_t *current = json_incref(request);
    *response_code = 200; // Default
    
    PipelineStep *step = pipeline;
    while (step) {
        if (strcmp(step->plugin, "result") == 0) {
            // Handle result step
            ASTNode *result_node = (ASTNode*)step->value;
            
            // Determine which condition to execute based on current state
            ResultCondition *condition = result_node->data.result_step.conditions;
            ResultCondition *selected_condition = NULL;
            
            // Check for error conditions first
            json_t *error = json_object_get(current, "error");
            if (error && json_is_string(error)) {
                // Look for error condition
                while (condition) {
                    if (strcmp(condition->condition_name, "error") == 0 ||
                        strcmp(condition->condition_name, "validationError") == 0) {
                        selected_condition = condition;
                        break;
                    }
                    condition = condition->next;
                }
            }
            
            // If no error condition found, use default "ok" condition
            if (!selected_condition) {
                condition = result_node->data.result_step.conditions;
                while (condition) {
                    if (strcmp(condition->condition_name, "ok") == 0) {
                        selected_condition = condition;
                        break;
                    }
                    condition = condition->next;
                }
            }
            
            if (selected_condition) {
                *response_code = selected_condition->status_code;
                
                // Execute the selected condition's pipeline
                if (selected_condition->pipeline) {
                    json_t *condition_result = NULL;
                    int temp_code;
                    int result = execute_pipeline_with_result(selected_condition->pipeline, 
                                                            current, arena, &condition_result, &temp_code);
                    if (result == 0 && condition_result) {
                        // json_decref(current);
                        current = condition_result;
                    }
                }
            }
            
            *final_response = current;
            return 0;
        }
        
        Plugin *plugin = find_plugin(step->plugin);
        if (!plugin) {
            fprintf(stderr, "Plugin not found: %s\n", step->plugin);
            // json_decref(current);
            return -1;
        }
        
        const char *config = step->value;
        if (step->is_variable) {
            // Look up variable value
            json_t *var_value = json_object_get(runtime->variables, step->value);
            if (var_value && json_is_string(var_value)) {
                config = json_string_value(var_value);
            }
        }
        
        json_t *result = plugin->execute(current, arena, arena_alloc_wrapper, arena_free_wrapper, config);
        if (!result) {
            fprintf(stderr, "Plugin %s failed\n", step->plugin);
            // json_decref(current);
            return -1;
        }
        
        // json_decref(current);
        current = result;
        step = step->next;
    }
    
    *final_response = current;
    return 0;
}

int execute_pipeline(PipelineStep *pipeline, json_t *request, MemoryArena *arena) {
    json_t *response = NULL;
    int response_code;
    int result = execute_pipeline_with_result(pipeline, request, arena, &response, &response_code);
    if (response) {
        // json_decref(response);
    }
    return result;
}

bool match_route(const char *pattern, const char *url, json_t *params) {
    // Simple route matching - could be enhanced
    char *pattern_copy = strdup(pattern);
    char *url_copy = strdup(url);
    
    char *pattern_part = strtok(pattern_copy, "/");
    char *url_part = strtok(url_copy, "/");
    
    while (pattern_part && url_part) {
        if (pattern_part[0] == ':') {
            // Parameter
            json_object_set_new(params, pattern_part + 1, json_string(url_part));
        } else if (strcmp(pattern_part, url_part) != 0) {
            free(pattern_copy);
            free(url_copy);
            return false;
        }
        
        pattern_part = strtok(NULL, "/");
        url_part = strtok(NULL, "/");
    }
    
    bool match = (pattern_part == NULL && url_part == NULL);
    free(pattern_copy);
    free(url_copy);
    return match;
}

static enum MHD_Result handle_request(void *cls, struct MHD_Connection *connection,
                         const char *url, const char *method,
                         const char *version, const char *upload_data,
                         size_t *upload_data_size, void **con_cls) {
    
    if (*con_cls == NULL) {
        *con_cls = (void *)1;
        return MHD_YES;
    }
    
    MemoryArena *arena = arena_create(1024 * 1024); // 1MB arena
    set_current_arena(arena); // Set arena for this thread
    
    json_t *request = create_request_json(connection, url, method, 
                                         upload_data, upload_data_size);
    
    // Find matching route
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_ROUTE_DEFINITION) {
            if (strcmp(stmt->data.route_def.method, method) == 0) {
                json_t *params = json_object_get(request, "params");
                if (match_route(stmt->data.route_def.route, url, params)) {
                    // Execute pipeline with result handling
                    json_t *final_response = NULL;
                    int response_code = 200;
                    
                    int result = execute_pipeline_with_result(stmt->data.route_def.pipeline, 
                                                            request, arena, &final_response, &response_code);
                    
                    if (result == 0 && final_response) {
                        // Convert JSON response to string - use regular malloc to avoid arena issues
                        set_current_arena(NULL); // Temporarily disable arena for json_dumps
                        char *response_str = json_dumps(final_response, JSON_COMPACT);
                        set_current_arena(arena); // Re-enable arena
                        
                        struct MHD_Response *mhd_response = 
                            MHD_create_response_from_buffer(strlen(response_str),
                                                           (void*)response_str,
                                                           MHD_RESPMEM_MUST_FREE);
                        
                        // Add JSON content type header
                        MHD_add_response_header(mhd_response, "Content-Type", "application/json");
                        
                        (void)MHD_queue_response(connection, response_code, mhd_response);
                        MHD_destroy_response(mhd_response);
                        
                        // json_decref(final_response);
                    } else {
                        // Error in pipeline execution
                        const char *error_response = "{\"error\": \"Internal server error\"}";
                        struct MHD_Response *mhd_response = 
                            MHD_create_response_from_buffer(strlen(error_response),
                                                           (void*)error_response,
                                                           MHD_RESPMEM_PERSISTENT);
                        (void)MHD_queue_response(connection, MHD_HTTP_INTERNAL_SERVER_ERROR, mhd_response);
                        MHD_destroy_response(mhd_response);
                    }
                    
                    // Clear current arena before freeing to prevent jansson from accessing freed memory
                    set_current_arena(NULL);
                    arena_free(arena);
                    return MHD_YES;
                }
            }
        }
    }
    
    // No route found
    const char *response = "{\"error\": \"Not found\"}";
    struct MHD_Response *mhd_response = 
        MHD_create_response_from_buffer(strlen(response),
                                       (void*)response,
                                       MHD_RESPMEM_PERSISTENT);
    (void)MHD_queue_response(connection, MHD_HTTP_NOT_FOUND, mhd_response);
    MHD_destroy_response(mhd_response);
    
    // Clear current arena before freeing to prevent jansson from accessing freed memory
    set_current_arena(NULL);
    arena_free(arena);
    return MHD_YES;
}

// Runtime initialization
int wp_runtime_init(const char *wp_file) {
    printf("Initializing runtime\n");
    
    // Check if we can access microhttpd functions
    printf("Checking microhttpd availability...\n");
    
    runtime = malloc(sizeof(WPRuntime));
    runtime->plugins = NULL;
    runtime->plugin_count = 0;
    runtime->variables = json_object();
    
    // Set up jansson to use arena allocators
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    // Parse wp file
    FILE *file = fopen(wp_file, "r");
    if (!file) {
        fprintf(stderr, "Error: Could not open file '%s'\n", wp_file);
        return -1;
    }
    
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    char *source = malloc((size_t)file_size + 1);
    fread(source, 1, (size_t)file_size, file);
    source[file_size] = '\0';
    fclose(file);

    printf("Tokenizing and parsing\n");
    
    // Tokenize and parse
    int token_count;
    Token *tokens = lexer_tokenize(source, &token_count);
    Parser *parser = parser_new(tokens, token_count);
    runtime->program = parser_parse(parser);

    printf("Parsed program\n");
    
    // Process variable assignments
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_VARIABLE_ASSIGNMENT) {
            json_object_set_new(runtime->variables, stmt->data.var_assign.name,
                               json_string(stmt->data.var_assign.value));
        }
    }
    
    // Load required plugins
    printf("Loading plugins...\n");
    if (load_plugin("jq") != 0) {
        printf("Warning: Failed to load jq plugin\n");
    } else {
        printf("Loaded jq plugin successfully\n");
    }
    
    if (load_plugin("lua") != 0) {
        printf("Warning: Failed to load lua plugin\n");
    } else {
        printf("Loaded lua plugin successfully\n");
    }
    
    if (load_plugin("pg") != 0) {
        printf("Warning: Failed to load pg plugin\n");
    } else {
        printf("Loaded pg plugin successfully\n");
    }
    
    // Start HTTP server
    printf("Starting HTTP server on port 8080...\n");
    
    // Try to start the daemon with more detailed error handling
    runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                      8080, NULL, NULL,
                                      &handle_request, NULL,
                                      MHD_OPTION_END);
    
    if (!runtime->daemon) {
        fprintf(stderr, "Error starting HTTP server on port 8080\n");
        
        // Try alternative port
        printf("Trying port 8081...\n");
        runtime->daemon = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                          8081, NULL, NULL,
                                          &handle_request, NULL,
                                          MHD_OPTION_END);
        
        if (!runtime->daemon) {
            fprintf(stderr, "Error starting HTTP server on port 8081 as well\n");
            fprintf(stderr, "Check if ports are in use or if you have permission to bind to them\n");
            return -1;
        } else {
            printf("HTTP server started successfully on port 8081\n");
        }
    } else {
        printf("HTTP server started successfully on port 8080\n");
    }
    
    parser_free(parser);
    free_tokens(tokens, token_count);
    free(source);
    
    return 0;
}

void wp_runtime_cleanup() {
    if (runtime) {
        if (runtime->daemon) {
            MHD_stop_daemon(runtime->daemon);
        }
        
        // Cleanup plugins
        for (int i = 0; i < runtime->plugin_count; i++) {
            dlclose(runtime->plugins[i].handle);
            free(runtime->plugins[i].name);
        }
        free(runtime->plugins);
        
        // json_decref(runtime->variables);
        free_ast(runtime->program);
        free(runtime);
    }
}

int main(int argc, char *argv[]) {
    if (argc != 2) {
        fprintf(stderr, "Usage: %s <wp_file>\n", argv[0]);
        return 1;
    }
    
    if (wp_runtime_init(argv[1]) != 0) {
        return 1;
    }
    
    printf("WP Runtime started on port 8080\n");
    printf("Press Enter to stop...\n");
    getchar();
    
    wp_runtime_cleanup();
    return 0;
}
