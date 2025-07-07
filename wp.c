#include <ctype.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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
  TOKEN_EQUALS
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
  AST_VARIABLE_ASSIGNMENT
} ASTNodeType;

// Forward declarations
typedef struct ASTNode ASTNode;
typedef struct PipelineStep PipelineStep;

// Pipeline step
struct PipelineStep {
  char *plugin;
  char *value;
  bool is_variable;
  PipelineStep *next;
};

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
  if (lexer->current >= strlen(lexer->source))
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
  char *value = malloc(length + 1);
  strncpy(value, lexer->source + start, length);
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
  char *value = malloc(length + 1);
  strncpy(value, lexer->source + start, length);
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
  char *value = malloc(length + 1);
  strncpy(value, lexer->source + start, length);
  value[length] = '\0';

  Token token = lexer_make_token(lexer, TOKEN_ROUTE, value);
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

  if (c == '`') {
    return lexer_read_string(lexer);
  }

  if (c == '/') {
    return lexer_read_route(lexer);
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

// Stringify functions
void stringify_indent(FILE *out, int level) {
  for (int i = 0; i < level * 2; i++) {
    fprintf(out, " ");
  }
}

void stringify_pipeline(FILE *out, PipelineStep *pipeline, int level) {
  PipelineStep *step = pipeline;
  while (step) {
    stringify_indent(out, level);
    fprintf(out, "|> %s: ", step->plugin);
    if (step->is_variable) {
      fprintf(out, "%s", step->value);
    } else {
      fprintf(out, "`%s`", step->value);
    }
    fprintf(out, "\n");
    step = step->next;
  }
}

void stringify_node(FILE *out, ASTNode *node, int level) {
  if (!node)
    return;

  switch (node->type) {
  case AST_PROGRAM:
    for (int i = 0; i < node->data.program.statement_count; i++) {
      stringify_node(out, node->data.program.statements[i], level);
      if (i < node->data.program.statement_count - 1) {
        fprintf(out, "\n");
      }
    }
    break;

  case AST_ROUTE_DEFINITION:
    fprintf(out, "%s %s\n", node->data.route_def.method,
            node->data.route_def.route);
    stringify_pipeline(out, node->data.route_def.pipeline, 1);
    break;

  case AST_VARIABLE_ASSIGNMENT:
    fprintf(out, "%s %s = `%s`\n", node->data.var_assign.plugin,
            node->data.var_assign.name, node->data.var_assign.value);
    break;
  }
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
  }

  free(node);
}

void free_tokens(Token *tokens, int count) {
  for (int i = 0; i < count; i++) {
    free(tokens[i].value);
  }
  free(tokens);
}

// Main function
int main(int argc, char *argv[]) {
  if (argc != 3 || strcmp(argv[1], "-f") != 0) {
    fprintf(stderr, "Usage: %s -f <filename>\n", argv[0]);
    return 1;
  }

  // Read file
  FILE *file = fopen(argv[2], "r");
  if (!file) {
    fprintf(stderr, "Error: Could not open file '%s'\n", argv[2]);
    return 1;
  }

  fseek(file, 0, SEEK_END);
  long file_size = ftell(file);
  fseek(file, 0, SEEK_SET);

  char *source = malloc(file_size + 1);
  fread(source, 1, file_size, file);
  source[file_size] = '\0';
  fclose(file);

  // Tokenize
  int token_count;
  Token *tokens = lexer_tokenize(source, &token_count);

  // Parse
  Parser *parser = parser_new(tokens, token_count);
  ASTNode *ast = parser_parse(parser);

  // Stringify and output
  stringify_node(stdout, ast, 0);

  // Cleanup
  parser_free(parser);
  free_ast(ast);
  free_tokens(tokens, token_count);
  free(source);

  return 0;
}