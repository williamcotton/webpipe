// Register WebPipe language definition
hljs.registerLanguage('webpipe', function(hljs) {
  return {
    name: 'WebPipe',
    case_insensitive: false,
    contains: [
      // Comments
      hljs.HASH_COMMENT_MODE,

      // HTTP Methods
      {
        scope: 'keyword',
        match: '\\b(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\\b'
      },

      // Config / pipeline keywords
      {
        scope: 'keyword',
        match: '\\b(config|pipeline)\\b'
      },

      {
        scope: 'keyword',
        match: '^\\s*[a-zA-Z_][a-zA-Z0-9_]*(?=\\s+[a-zA-Z_][a-zA-Z0-9_]*\\s*=)'
      },

      // Pipeline operator
      {
        scope: 'operator',
        match: '\\|>'
      },

      // Middleware functions in pipelines (word before colon)
      {
        scope: 'keyword',
        match: '\\b[a-zA-Z_][a-zA-Z0-9_]*(?=\\s*:)'
      },

      // Route paths
      {
        scope: 'string',
        match: '/[^\\s]*',
        contains: [
          {
            scope: 'variable',
            match: ':[a-zA-Z_][a-zA-Z0-9_]*'
          }
        ]
      },

      // Lua embedded content
      {
        begin: '\\blua:\\s*`',
        end: '`',
        subLanguage: 'lua',
        contains: [
          {
            scope: 'variable',
            match: 'request\\.[a-zA-Z_][a-zA-Z0-9_]*'
          }
        ]
      },

      // JQ embedded content
      {
        begin: '\\bjq:\\s*`',
        end: '`',
        subLanguage: 'json',
        contains: [
          {
            scope: 'variable',
            match: '\\.[a-zA-Z_][a-zA-Z0-9_]*'
          }
        ]
      },

      // SQL embedded content
      {
        begin: '\\bpg:\\s*`',
        end: '`',
        subLanguage: 'sql'
      },

      // Generic back‑tick strings
      {
        scope: 'string',
        begin: '`',
        end: '`',
        contains: [
          {
            scope: 'variable',
            match: '\\.[a-zA-Z_][a-zA-Z0-9_]*'
          },
          hljs.QUOTE_STRING_MODE
        ]
      },

      // Regular quoted strings
      hljs.QUOTE_STRING_MODE,

      // Numbers
      hljs.C_NUMBER_MODE,

      // Environment variables
      {
        scope: 'variable',
        match: '\\$[a-zA-Z_][a-zA-Z0-9_]*'
      },

      // Boolean literals
      {
        scope: 'literal',
        match: '\\b(true|false)\\b'
      },

      // BDD test keywords with quoted strings
      {
        begin: '\\b(describe|it)\\s+',
        beginScope: 'keyword',
        end: '$',
        contains: [
          {
            scope: 'string',
            match: '"[^"]*"'
          }
        ]
      },

      // Other BDD keywords
      {
        scope: 'keyword',
        match: '\\b(with|when|then|and|executing|calling|input|output|equals|status|mock|returning)\\b'
      }
    ]
  };
});
hljs.highlightAll();