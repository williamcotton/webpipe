#ifndef WP_H
#define WP_H

#include <stdbool.h>
#include <jansson.h>
#include <microhttpd.h>

// Forward declarations
typedef struct ASTNode ASTNode;
typedef struct PipelineStep PipelineStep;

// Memory arena for per-request allocations
typedef struct MemoryArena {
    char *memory;
    size_t size;
    size_t used;
} MemoryArena;

// Parse context for arena-based parser memory management
typedef struct ParseContext {
    MemoryArena *parse_arena;    // For AST nodes, strings, conditions
    MemoryArena *runtime_arena;  // For long-lived runtime data
} ParseContext;

// Token types
typedef enum {
  TOKEN_EOF,
  TOKEN_NEWLINE,
  TOKEN_HTTP_METHOD,
  TOKEN_ROUTE,
  TOKEN_PIPE,
  TOKEN_IDENTIFIER,
  TOKEN_STRING,
  TOKEN_COLON,
  TOKEN_EQUALS,
  TOKEN_LBRACE,
  TOKEN_RBRACE,
  TOKEN_LPAREN,
  TOKEN_RPAREN,
  TOKEN_NUMBER,
  TOKEN_CONFIG,
  TOKEN_DOLLAR,
  TOKEN_OR,
  TOKEN_COMMA,
  TOKEN_LBRACKET,
  TOKEN_RBRACKET,
  TOKEN_TRUE,
  TOKEN_FALSE,
  TOKEN_NULL,
  TOKEN_COMMENT
} TokenType;

// Token structure
typedef struct {
  TokenType type;
  char *value;
  int line;
  int column;
} Token;

// AST Node types
typedef enum {
  AST_PROGRAM,
  AST_ROUTE_DEFINITION,
  AST_PIPELINE_STEP,
  AST_VARIABLE_ASSIGNMENT,
  AST_RESULT_STEP,
  AST_CONFIG_BLOCK,
  AST_CONFIG_VALUE_STRING,
  AST_CONFIG_VALUE_NUMBER,
  AST_CONFIG_VALUE_BOOLEAN,
  AST_CONFIG_VALUE_NULL,
  AST_CONFIG_VALUE_ENV_CALL,
  AST_CONFIG_VALUE_OBJECT,
  AST_CONFIG_VALUE_ARRAY
} ASTNodeType;

// Pipeline step
struct PipelineStep {
  char *middleware;
  char *value;
  bool is_variable;
  PipelineStep *next;
};

// Result condition structure
typedef struct ResultCondition {
  char *condition_name;
  int status_code;
  PipelineStep *pipeline;
  struct ResultCondition *next;
} ResultCondition;

// Configuration value structures
typedef struct ConfigProperty {
  char *key;
  ASTNode *value;
  struct ConfigProperty *next;
} ConfigProperty;

typedef struct ConfigArrayItem {
  ASTNode *value;
  struct ConfigArrayItem *next;
} ConfigArrayItem;

// AST Node
struct ASTNode {
  ASTNodeType type;
  union {
    struct {
      ASTNode **statements;
      int statement_count;
    } program;
    struct {
      char *method;
      char *route;
      PipelineStep *pipeline;
    } route_def;
    struct {
      char *middleware;
      char *name;
      char *value;
    } var_assign;
    struct {
      ResultCondition *conditions;
    } result_step;
    struct {
      char *name;
      ConfigProperty *properties;
    } config_block;
    struct {
      char *value;
    } config_value_string;
    struct {
      double value;
      bool is_integer;
    } config_value_number;
    struct {
      bool value;
    } config_value_boolean;
    struct {
      char *env_var;
      char *default_value;
    } config_value_env_call;
    struct {
      ConfigProperty *properties;
    } config_value_object;
    struct {
      ConfigArrayItem *items;
    } config_value_array;
  } data;
};

// Lexer state
typedef struct {
  const char *source;
  int current;
  int line;
  int column;
} Lexer;

// Parser state
typedef struct {
  Token *tokens;
  int token_count;
  int current;
  ParseContext *ctx;  // Parse context for arena allocation
} Parser;

// Arena allocation function types for middleware
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Middleware interface with arena functions
typedef struct {
    char *name;
    void *handle;
    json_t *(*execute)(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, json_t *middleware_config, char **contentType, json_t *variables);
} Middleware;

// Function declarations
char *strdup_safe(const char *s);
void set_current_arena(MemoryArena *arena);
MemoryArena *get_current_arena(void);
void *jansson_arena_malloc(size_t size);
void jansson_arena_free(void *ptr);
MemoryArena *arena_create(size_t size);
void *arena_alloc(MemoryArena *arena, size_t size);
void arena_free(MemoryArena *arena);
char *arena_strdup(MemoryArena *arena, const char *str);
char *arena_strndup(MemoryArena *arena, const char *str, size_t n);
ParseContext *parse_context_create(void);
void parse_context_destroy(ParseContext *ctx);
Lexer *lexer_new(const char *source);
void lexer_free(Lexer *lexer);
char lexer_peek(Lexer *lexer);
char lexer_advance(Lexer *lexer);
void lexer_skip_whitespace(Lexer *lexer);
Token lexer_make_token(Lexer *lexer, TokenType type, const char *value);
Token lexer_read_string(Lexer *lexer);
Token lexer_read_identifier(Lexer *lexer);
Token lexer_read_route(Lexer *lexer);
Token lexer_read_number(Lexer *lexer);
Token lexer_read_comment(Lexer *lexer);
Token lexer_next_token(Lexer *lexer);
Token *lexer_tokenize(const char *source, int *token_count);
Parser *parser_new(Token *tokens, int token_count);
Parser *parser_new_with_context(Token *tokens, int token_count, ParseContext *ctx);
void parser_free(Parser *parser);
ASTNode *parser_parse(Parser *parser);
ASTNode *parser_parse_result_step(Parser *parser);
void free_ast(ASTNode *node);
void free_tokens(Token *tokens, int count);
void stringify_node(FILE *out, ASTNode *node, int level);
int wp_runtime_init(const char *wp_file, int port);
void wp_runtime_cleanup(void);

// Internal function declarations (static functions that need prototypes)
bool parser_is_at_end(Parser *parser);
Token *parser_peek(Parser *parser);
Token *parser_advance(Parser *parser);
bool parser_check(Parser *parser, TokenType type);
bool parser_match(Parser *parser, TokenType type);
void parser_consume_newlines(Parser *parser);
PipelineStep *parser_parse_pipeline(Parser *parser);
ASTNode *parser_parse_route_definition(Parser *parser);
ASTNode *parser_parse_variable_assignment(Parser *parser);
ASTNode *parser_parse_config_block(Parser *parser);
ASTNode *parser_parse_config_value(Parser *parser);
ConfigProperty *parser_parse_config_properties(Parser *parser);
json_t *config_ast_to_json(ASTNode *node);
json_t *config_block_to_json(ASTNode *config_block);
ASTNode *parser_parse_statement(Parser *parser);
void stringify_indent(FILE *out, int level);
void stringify_pipeline(FILE *out, PipelineStep *pipeline, int level);
void free_pipeline(PipelineStep *pipeline);
void free_result_conditions(ResultCondition *conditions);

// Structure to hold POST data during processing
typedef struct {
    uint32_t magic;  // Magic number to identify PostData structures
    MemoryArena *arena;
    char *post_data;
    size_t post_data_size;
    size_t post_data_capacity;
    struct MHD_PostProcessor *post_processor;
    json_t *form_data;  // Parsed form data as JSON object
    int is_form_data;   // 1 if this is form data, 0 if raw data
} PostData;

#define POST_DATA_MAGIC 0x504F5354  // "POST" in ASCII

// Server internal function declarations
int load_middleware(const char *name);
Middleware *find_middleware(const char *name);
void collect_middleware_names_from_ast(ASTNode *node, char **middleware_names, int *middleware_count, int max_middleware);
json_t *create_request_json(struct MHD_Connection *connection, 
                           const char *url, const char *method,
                           PostData *post_data);
int execute_pipeline_with_result(PipelineStep *pipeline, json_t *request, MemoryArena *arena, 
                                json_t **final_response, int *response_code, char **content_type);
int execute_pipeline(PipelineStep *pipeline, json_t *request, MemoryArena *arena);
bool match_route(const char *pattern, const char *url, json_t *params);

// Cookie parsing function
json_t *parse_cookies(const char *cookie_header);

#endif // WP_H
