#ifndef WP_H
#define WP_H

#include <stdbool.h>
#include <jansson.h>

// Forward declarations
typedef struct ASTNode ASTNode;
typedef struct PipelineStep PipelineStep;

// Memory arena for per-request allocations
typedef struct MemoryArena {
    char *memory;
    size_t size;
    size_t used;
} MemoryArena;

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
  TOKEN_NUMBER
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
  AST_RESULT_STEP
} ASTNodeType;

// Pipeline step
struct PipelineStep {
  char *plugin;
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
      char *plugin;
      char *name;
      char *value;
    } var_assign;
    struct {
      ResultCondition *conditions;
    } result_step;
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
} Parser;

// Function declarations
char *strdup_safe(const char *s);
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
Token lexer_next_token(Lexer *lexer);
Token *lexer_tokenize(const char *source, int *token_count);
Parser *parser_new(Token *tokens, int token_count);
void parser_free(Parser *parser);
ASTNode *parser_parse(Parser *parser);
ASTNode *parser_parse_result_step(Parser *parser);
void free_ast(ASTNode *node);
void free_tokens(Token *tokens, int count);
void stringify_node(FILE *out, ASTNode *node, int level);
int wp_runtime_init(const char *wp_file);
void wp_runtime_cleanup(void);

#endif // WP_H
