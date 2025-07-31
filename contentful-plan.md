# Contentful Rich Text Integration Plan

## Overview
Integrate Contentful's rich text parsing capabilities into WebPipe using a four-stage pipeline approach: Lua → Fetch → Lua → Mustache. This enables WebPipe to serve as a headless CMS frontend for Contentful with full rich text rendering and templated HTML output.

## Architecture Approach
**Four-Stage Pipeline Pattern:**
1. **Lua Stage 1**: Build API request with environment variables (API keys, space ID)
2. **Fetch Stage**: Execute HTTP request to Contentful API
3. **Lua Stage 2**: Parse Contentful rich text response and prepare template data
4. **Mustache Stage**: Render final HTML using reusable templates

## Key Benefits
- **Clean Separation**: Data fetching, processing, and presentation are separate stages
- **Template Reusability**: Blog layouts, article templates shared across content
- **Rich HTML Structure**: Complete page layouts with navigation, headers, footers
- **Existing Integration**: Leverages WebPipe's robust mustache support with partials

## Implementation Example

```wp
GET /article/:slug
  # Stage 1: Build API request with credentials
  |> lua: `
    local spaceId = getEnv("CONTENTFUL_SPACE_ID")
    local token = getEnv("CONTENTFUL_ACCESS_TOKEN")
    return {
      fetchUrl = "https://cdn.contentful.com/spaces/" .. spaceId .. "/entries?fields.slug=" .. request.params.slug,
      fetchHeaders = { ["Authorization"] = "Bearer " .. token }
    }
  `
  
  # Stage 2: Fetch from Contentful API
  |> fetch: `placeholder`
  
  # Stage 3: Parse rich text and prepare template data
  |> lua: `
    local entry = request.data.response.items[1]
    return {
      title = entry.fields.title,
      bodyHtml = renderRichText(entry.fields.body), -- parsed rich text
      publishedAt = entry.fields.publishedAt,
      author = entry.fields.author.fields.name,
      seoDescription = entry.fields.seoDescription
    }
  `
  
  # Stage 4: Render with mustache template
  |> mustache: `
    {{<articleLayout}}
      {{$title}}{{title}}{{/title}}
      {{$metaDescription}}{{seoDescription}}{{/metaDescription}}
      {{$content}}
        <article>
          <header>
            <h1>{{title}}</h1>
            <div class="meta">By {{author}} on {{publishedAt}}</div>
          </header>
          <div class="content">{{{bodyHtml}}}</div>
        </article>
      {{/content}}
    {{/articleLayout}}
  `
```

## Key Components

### 1. Enhanced Lua Middleware
- Add `getEnv(varName, defaultValue)` function to Lua global scope
- Enable secure access to environment variables for API credentials
- Follow existing pattern established by `executeSql()` function

### 2. Rich Text Parser in Lua
- Recursive tree traversal for nested content structures
- Handle Contentful node types:
  - `BLOCKS.EMBEDDED_ASSET` → `<img>` tags with proper src/alt
  - `INLINES.ENTRY_HYPERLINK` → `<a>` tags with routing
  - `BLOCKS.EMBEDDED_ENTRY` → Code blocks with syntax highlighting
  - `BLOCKS.PARAGRAPH`, `BLOCKS.HEADING_*`, etc. → Standard HTML elements
- Transform rich text JSON to semantic HTML
- Support for embedded assets, entry links, and cross-references

### 3. Template Integration
- Reusable mustache partials for page layouts
- Article templates, blog post layouts, landing pages
- SEO-friendly meta tag integration
- Responsive design support

### 4. Configuration & Security
- Environment variable management for API keys
- Content Delivery vs Preview API support
- Whitelist approach for environment variable access
- Error handling and fallback content

## Rich Text Node Rendering

The Lua rich text parser will handle these Contentful node types:

```lua
function renderNode(node)
  local nodeType = node.nodeType
  
  if nodeType == "embedded-asset-block" then
    local asset = node.data.target.fields
    return string.format('<img src="%s" alt="%s" class="embedded-asset" />', 
                       asset.file.url, asset.title or "")
  
  elseif nodeType == "embedded-entry-block" then
    if node.data.target.sys.contentType.sys.id == "codeBlock" then
      return string.format('<pre><code class="language-%s">%s</code></pre>',
                         node.data.target.fields.lang,
                         escapeHtml(node.data.target.fields.code))
    end
  
  elseif nodeType == "entry-hyperlink" then
    local target = node.data.target
    local href = generateEntryUrl(target)
    return string.format('<a href="%s">%s</a>', href, renderRichText(node))
  
  elseif nodeType == "paragraph" then
    return "<p>" .. renderRichText(node) .. "</p>"
  
  elseif nodeType == "heading-1" then
    return "<h1>" .. renderRichText(node) .. "</h1>"
    
  -- Additional node types...
  end
  
  return ""
end
```

## Template Structure

```wp
mustache articleLayout = `
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>{{$title}}Default Title{{/title}}</title>
  <meta name="description" content="{{$metaDescription}}{{/metaDescription}}">
  <link rel="stylesheet" href="/styles/article.css">
</head>
<body>
  {{>header}}
  <main>
    {{$content}}Default content{{/content}}
  </main>
  {{>footer}}
</body>
</html>
`

mustache blogLayout = `
{{<articleLayout}}
  {{$content}}
    <div class="blog-container">
      {{$blogContent}}{{/blogContent}}
      {{>sidebar}}
    </div>
  {{/content}}
{{/articleLayout}}
`
```

## Implementation Phases

### Phase 1: Core Lua Enhancement
- **Goal**: Add environment variable access to Lua middleware
- **Tasks**:
  - Implement `getEnv()` function in `src/middleware/lua.c`
  - Add security whitelist for allowed environment variables
  - Test basic environment variable access in pipeline
- **Deliverable**: Lua can securely access CONTENTFUL_SPACE_ID, CONTENTFUL_ACCESS_TOKEN
- **Validation**: Simple test route that displays environment variables

### Phase 2: Basic Contentful Integration 
- **Goal**: Complete four-stage pipeline with raw data
- **Tasks**:
  - Create example routes using Lua → Fetch → Lua → Mustache pattern
  - Test API authentication and data retrieval
  - Verify JSON response structure from Contentful
  - Create basic mustache templates for content display
- **Deliverable**: Working Contentful API integration with templated HTML output
- **Validation**: Blog post route that displays raw Contentful data in HTML template

### Phase 3: Rich Text Parsing Engine
- **Goal**: Transform Contentful rich text to semantic HTML
- **Tasks**:
  - Implement recursive rich text parser in Lua
  - Handle core node types (paragraphs, headings, lists, bold, italic)
  - Add support for embedded assets (images) with proper attributes
  - Create reusable Lua functions for content rendering
  - Add basic error handling for malformed content
- **Deliverable**: Rich text content renders as proper semantic HTML
- **Validation**: Article with complex rich text (images, formatting) displays correctly

### Phase 4: Advanced Features & Production Polish
- **Goal**: Production-ready Contentful CMS integration
- **Tasks**:
  - Add entry hyperlink support with proper routing
  - Implement embedded entry handling (code blocks, quotes, etc.)
  - Add code block syntax highlighting support
  - Implement content preview mode for draft content
  - Create comprehensive mustache template library
  - Add SEO optimization (meta tags, structured data)
  - Implement error handling and fallback content
  - Create test suite with various content types
  - Document usage patterns and best practices
- **Deliverable**: Complete Contentful CMS integration ready for production
- **Validation**: Full blog/CMS site with navigation, SEO, error handling

## Success Criteria

- ✅ Secure environment variable access in Lua pipelines
- ✅ Successful authentication and data retrieval from Contentful API
- ✅ Accurate parsing of all major Contentful rich text node types
- ✅ Semantic HTML output with proper accessibility attributes
- ✅ Reusable template system for different content types
- ✅ Production-ready error handling and fallbacks
- ✅ SEO-optimized output with meta tags and structured data
- ✅ Performance suitable for production traffic

This phased approach ensures we can validate each component before building on it, with working functionality available after each phase.