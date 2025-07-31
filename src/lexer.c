#include <ctype.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wp.h"

// Helper functions
char *strdup_safe(const char *s) {
  if (!s)
    return NULL;
  size_t len = strlen(s);
  char *copy = malloc(len + 1);
  if (copy) {
    memcpy(copy, s, len);
    copy[len] = '\0';
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
  while (isspace(lexer_peek(lexer))) {
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
  char quote_char = lexer_peek(lexer); // Remember the quote character (` or ")
  bool is_triple_quote = false;
  
  // Check for triple double quotes
  if (quote_char == '"' && 
      lexer->source[lexer->current + 1] == '"' && 
      lexer->source[lexer->current + 2] == '"') {
    is_triple_quote = true;
    lexer_advance(lexer); // Skip first "
    lexer_advance(lexer); // Skip second "
    lexer_advance(lexer); // Skip third "
  } else {
    lexer_advance(lexer); // Skip opening quote
  }
  
  int start = lexer->current;

  if (is_triple_quote) {
    // For triple quotes, look for the closing """
    while (lexer_peek(lexer) != '\0') {
      if (lexer_peek(lexer) == '"' && 
          lexer->source[lexer->current + 1] == '"' && 
          lexer->source[lexer->current + 2] == '"') {
        break;
      }
      lexer_advance(lexer);
    }
  } else {
    // Original logic for single quotes
    while (lexer_peek(lexer) != '\0') {
      if (lexer_peek(lexer) == '\\') {
        lexer_advance(lexer); // Skip escape character
        if (lexer_peek(lexer) != '\0') {
          lexer_advance(lexer); // Skip escaped character
        }
      } else if (lexer_peek(lexer) == quote_char) {
        break;
      } else {
        lexer_advance(lexer);
      }
    }
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  if (is_triple_quote) {
    lexer_advance(lexer); // Skip first closing "
    lexer_advance(lexer); // Skip second closing "
    lexer_advance(lexer); // Skip third closing "
  } else {
    lexer_advance(lexer); // Skip closing quote
  }

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
      strcmp(value, "PUT") == 0 || strcmp(value, "DELETE") == 0 ||
      strcmp(value, "PATCH") == 0) {
    type = TOKEN_HTTP_METHOD;
  } else if (strcmp(value, "config") == 0) {
    type = TOKEN_CONFIG;
  } else if (strcmp(value, "pipeline") == 0) {
    type = TOKEN_IDENTIFIER; // Keep as identifier, we'll handle it in parser
  } else if (strcmp(value, "true") == 0) {
    type = TOKEN_TRUE;
  } else if (strcmp(value, "false") == 0) {
    type = TOKEN_FALSE;
  } else if (strcmp(value, "null") == 0) {
    type = TOKEN_NULL;
  } else if (strcmp(value, "describe") == 0) {
    type = TOKEN_DESCRIBE;
  } else if (strcmp(value, "it") == 0) {
    type = TOKEN_IT;
  } else if (strcmp(value, "with") == 0) {
    type = TOKEN_WITH;
  } else if (strcmp(value, "mock") == 0) {
    type = TOKEN_MOCK;
  } else if (strcmp(value, "returning") == 0) {
    type = TOKEN_RETURNING;
  } else if (strcmp(value, "when") == 0) {
    type = TOKEN_WHEN;
  } else if (strcmp(value, "executing") == 0) {
    type = TOKEN_EXECUTING;
  } else if (strcmp(value, "variable") == 0) {
    type = TOKEN_VARIABLE;
  } else if (strcmp(value, "calling") == 0) {
    type = TOKEN_CALLING;
  } else if (strcmp(value, "input") == 0) {
    type = TOKEN_INPUT;
  } else if (strcmp(value, "then") == 0) {
    type = TOKEN_THEN;
  } else if (strcmp(value, "output") == 0) {
    type = TOKEN_OUTPUT;
  } else if (strcmp(value, "equals") == 0) {
    type = TOKEN_EQUALS_ASSERTION;
  } else if (strcmp(value, "status") == 0) {
    type = TOKEN_STATUS;
  } else if (strcmp(value, "is") == 0) {
    type = TOKEN_IS;
  } else if (strcmp(value, "and") == 0) {
    type = TOKEN_AND;
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
    if (isalnum(c) || c == '/' || c == ':' || c == '-' || c == '_' || c == '.') {
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

  // Handle decimal numbers
  if (lexer_peek(lexer) == '.') {
    lexer_advance(lexer); // consume '.'
    while (isdigit(lexer_peek(lexer))) {
      lexer_advance(lexer);
    }
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  Token token = lexer_make_token(lexer, TOKEN_NUMBER, value);
  free(value);
  return token;
}

Token lexer_read_comment(Lexer *lexer) {
  lexer_advance(lexer); // Skip '#'
  int start = lexer->current;

  // Read until end of line or end of file
  while (lexer_peek(lexer) != '\0' && lexer_peek(lexer) != '\n') {
    lexer_advance(lexer);
  }

  int length = lexer->current - start;
  char *value = malloc((size_t)length + 1);
  strncpy(value, lexer->source + start, (size_t)length);
  value[length] = '\0';

  Token token = lexer_make_token(lexer, TOKEN_COMMENT, value);
  free(value);
  return token;
}

Token lexer_next_token(Lexer *lexer) {
  // Skip whitespace except newlines
  while (isspace(lexer_peek(lexer)) && lexer_peek(lexer) != '\n') {
    lexer_advance(lexer);
  }

  char c = lexer_peek(lexer);

  if (c == '\0') {
    return lexer_make_token(lexer, TOKEN_EOF, "");
  }

  if (c == '\n') {
    int line = lexer->line;
    int column = lexer->column;
    lexer_advance(lexer);
    Token token;
    token.type = TOKEN_NEWLINE;
    token.value = strdup_safe("\n");
    token.line = line;
    token.column = column;
    return token;
  }

  if (c == '|' && lexer->source[lexer->current + 1] == '>') {
    lexer_advance(lexer);
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_PIPE, "|>");
  }

  if (c == '|' && lexer->source[lexer->current + 1] == '|') {
    lexer_advance(lexer);
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_OR, "||");
  }

  if (c == '$') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_DOLLAR, "$");
  }

  if (c == ':') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_COLON, ":");
  }

  if (c == '=') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_EQUALS, "=");
  }

  if (c == '.') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_DOT, ".");
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

  if (c == '[') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_LBRACKET, "[");
  }

  if (c == ']') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_RBRACKET, "]");
  }

  if (c == ',') {
    lexer_advance(lexer);
    return lexer_make_token(lexer, TOKEN_COMMA, ",");
  }

  if (c == '`') {
    return lexer_read_string(lexer);
  }

  if (c == '"') {
    return lexer_read_string(lexer);
  }

  if (c == '#') {
    return lexer_read_comment(lexer);
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
  Token *tokens = malloc(sizeof(Token) * 10000); // Max 1000 tokens
  int count = 0;

  Token token;
  do {
    token = lexer_next_token(lexer);
    
    // Check if we would exceed the token array bounds
    if (count >= 10000) {
      fprintf(stderr, "Error: Source file has more than 10000 tokens\n");
      free(tokens);
      lexer_free(lexer);
      *token_count = 0;
      return NULL;
    }
    
    tokens[count++] = token;
  } while (token.type != TOKEN_EOF);

  *token_count = count;
  lexer_free(lexer);
  return tokens;
}
