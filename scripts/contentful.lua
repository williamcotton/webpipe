local contentful = {}

function contentful.escapeHtml(text)
  if not text then return "" end
  return text:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;"):gsub("\"", "&quot;"):gsub("'", "&#39;")
end

function contentful.generateEntryUrl(target)
  if not target or not target.fields then return "#" end
  local contentType = target.sys and target.sys.contentType and target.sys.contentType.sys and target.sys.contentType.sys.id
  local slug = target.fields.slug
  
  if contentType == "blogPost" and slug then
    return "/contentful/article/" .. slug
  elseif contentType == "page" and slug then
    return "/contentful/page/" .. slug
  else
    return "#"
  end
end

function contentful.renderTextWithMarks(textNode)
  if not textNode or textNode.nodeType ~= "text" then return "" end
  
  local value = contentful.escapeHtml(textNode.value or "")
  
  if textNode.marks then
    for _, mark in ipairs(textNode.marks) do
      if mark.type == "bold" then
        value = "<strong>" .. value .. "</strong>"
      elseif mark.type == "italic" then
        value = "<em>" .. value .. "</em>"
      elseif mark.type == "code" then
        value = "<code>" .. value .. "</code>"
      elseif mark.type == "underline" then
        value = "<u>" .. value .. "</u>"
      end
    end
  end
  
  return value
end

function contentful.renderRichText(richTextObj, includes)
  if not richTextObj or not richTextObj.content then
    return ""
  end
  
  -- Helper function to resolve entry references from includes
  local function resolveEntry(entryId)
    if not includes or not includes.Entry then return nil end
    for _, entry in ipairs(includes.Entry) do
      if entry.sys and entry.sys.id == entryId then
        return entry
      end
    end
    return nil
  end
  
  -- Helper function to resolve asset references from includes
  local function resolveAsset(assetId)
    if not includes or not includes.Asset then return nil end
    for _, asset in ipairs(includes.Asset) do
      if asset.sys and asset.sys.id == assetId then
        return asset
      end
    end
    return nil
  end
  
  local html = ""
  for _, node in ipairs(richTextObj.content) do
    
    -- Handle embedded assets (images)
    if node.nodeType == "embedded-asset-block" then
      local asset = nil
      
      -- Try to get asset from direct data first
      if node.data and node.data.target and node.data.target.fields then
        asset = node.data.target.fields
      -- Otherwise resolve from includes
      elseif node.data and node.data.target and node.data.target.sys then
        local assetId = node.data.target.sys.id
        local resolvedAsset = resolveAsset(assetId)
        if resolvedAsset and resolvedAsset.fields then
          asset = resolvedAsset.fields
        end
      end
      
      if asset then
        local url = asset.file and asset.file.url or ""
        local title = asset.title or ""
        local description = asset.description or title
        
        -- Ensure HTTPS URLs
        if url:sub(1, 2) == "//" then
          url = "https:" .. url
        end
        
        html = html .. string.format('<img src="%s" alt="%s" title="%s" class="embedded-asset" loading="lazy" />\n', 
                                   contentful.escapeHtml(url), contentful.escapeHtml(description), contentful.escapeHtml(title))
      end
      
    -- Handle embedded entries (code blocks, etc.)
    elseif node.nodeType == "embedded-entry-block" then
      if node.data and node.data.target and node.data.target.sys then
        local entryId = node.data.target.sys.id
        local resolvedEntry = resolveEntry(entryId)
        
        if resolvedEntry and resolvedEntry.fields then
          local contentType = resolvedEntry.sys and resolvedEntry.sys.contentType and resolvedEntry.sys.contentType.sys and resolvedEntry.sys.contentType.sys.id
          
          if (contentType == "codeBlock" or contentType == "codeSample") then
            local code = contentful.escapeHtml(resolvedEntry.fields.code or "")
            local lang = resolvedEntry.fields.lang or resolvedEntry.fields.language or "text"
            local filename = resolvedEntry.fields.filename and (" data-filename=\"" .. contentful.escapeHtml(resolvedEntry.fields.filename) .. "\"") or ""
            local title = resolvedEntry.fields.title and ("\n<!-- " .. contentful.escapeHtml(resolvedEntry.fields.title) .. " -->\n") or ""
            
            html = html .. title .. string.format('<pre class="code-block"><code class="language-%s"%s>%s</code></pre>\n', 
                                                 contentful.escapeHtml(lang), filename, code)
          end
        end
      end
      
    -- Handle paragraphs with inline content
    elseif node.nodeType == "paragraph" and node.content then
      html = html .. "<p>"
      for _, inline in ipairs(node.content) do
        if inline.nodeType == "text" then
          html = html .. contentful.renderTextWithMarks(inline)
        elseif inline.nodeType == "hyperlink" then
          local linkText = ""
          if inline.content then
            for _, linkContent in ipairs(inline.content) do
              linkText = linkText .. contentful.renderTextWithMarks(linkContent)
            end
          end
          local uri = inline.data and inline.data.uri or "#"
          html = html .. string.format('<a href="%s" target="_blank" rel="noopener noreferrer">%s</a>', 
                                      contentful.escapeHtml(uri), linkText)
        elseif inline.nodeType == "entry-hyperlink" then
          local linkText = ""
          if inline.content then
            for _, linkContent in ipairs(inline.content) do
              linkText = linkText .. contentful.renderTextWithMarks(linkContent)
            end
          end
          local target = inline.data and inline.data.target
          local href = contentful.generateEntryUrl(target)
          local title = target and target.fields and target.fields.title or ""
          
          html = html .. string.format('<a href="%s" title="%s">%s</a>', 
                                      contentful.escapeHtml(href), contentful.escapeHtml(title), linkText)
        elseif inline.nodeType == "asset-hyperlink" then
          local linkText = ""
          if inline.content then
            for _, linkContent in ipairs(inline.content) do
              linkText = linkText .. contentful.renderTextWithMarks(linkContent)
            end
          end
          
          local asset = nil
          local url = "#"
          
          -- Try to get asset from direct data first
          if inline.data and inline.data.target and inline.data.target.fields then
            asset = inline.data.target.fields
          -- Otherwise resolve from includes
          elseif inline.data and inline.data.target and inline.data.target.sys then
            local assetId = inline.data.target.sys.id
            local resolvedAsset = resolveAsset(assetId)
            if resolvedAsset and resolvedAsset.fields then
              asset = resolvedAsset.fields
            end
          end
          
          if asset and asset.file and asset.file.url then
            url = asset.file.url
            if url:sub(1, 2) == "//" then
              url = "https:" .. url
            end
          end
          
          html = html .. string.format('<a href="%s" download>%s</a>', contentful.escapeHtml(url), linkText)
        end
      end
      html = html .. "</p>\n"
      
    -- Handle headings
    elseif node.nodeType == "heading-1" and node.content then
      html = html .. "<h1>"
      for _, textNode in ipairs(node.content) do
        html = html .. contentful.renderTextWithMarks(textNode)
      end
      html = html .. "</h1>\n"
    elseif node.nodeType == "heading-2" and node.content then
      html = html .. "<h2>"
      for _, textNode in ipairs(node.content) do
        html = html .. contentful.renderTextWithMarks(textNode)
      end
      html = html .. "</h2>\n"
    elseif node.nodeType == "heading-3" and node.content then
      html = html .. "<h3>"
      for _, textNode in ipairs(node.content) do
        html = html .. contentful.renderTextWithMarks(textNode)
      end
      html = html .. "</h3>\n"
    elseif node.nodeType == "heading-4" and node.content then
      html = html .. "<h4>"
      for _, textNode in ipairs(node.content) do
        html = html .. contentful.renderTextWithMarks(textNode)
      end
      html = html .. "</h4>\n"
    elseif node.nodeType == "heading-5" and node.content then
      html = html .. "<h5>"
      for _, textNode in ipairs(node.content) do
        html = html .. contentful.renderTextWithMarks(textNode)
      end
      html = html .. "</h5>\n"
    elseif node.nodeType == "heading-6" and node.content then
      html = html .. "<h6>"
      for _, textNode in ipairs(node.content) do
        html = html .. contentful.renderTextWithMarks(textNode)
      end
      html = html .. "</h6>\n"
      
    -- Handle lists
    elseif node.nodeType == "unordered-list" and node.content then
      html = html .. "<ul>\n"
      for _, listItem in ipairs(node.content) do
        if listItem.nodeType == "list-item" and listItem.content then
          html = html .. "<li>"
          for _, itemContent in ipairs(listItem.content) do
            if itemContent.nodeType == "paragraph" and itemContent.content then
              for _, textNode in ipairs(itemContent.content) do
                html = html .. contentful.renderTextWithMarks(textNode)
              end
            else
              -- Recursively handle nested content
              html = html .. contentful.renderRichText(itemContent)
            end
          end
          html = html .. "</li>\n"
        end
      end
      html = html .. "</ul>\n"
    elseif node.nodeType == "ordered-list" and node.content then
      html = html .. "<ol>\n"
      for _, listItem in ipairs(node.content) do
        if listItem.nodeType == "list-item" and listItem.content then
          html = html .. "<li>"
          for _, itemContent in ipairs(listItem.content) do
            if itemContent.nodeType == "paragraph" and itemContent.content then
              for _, textNode in ipairs(itemContent.content) do
                html = html .. contentful.renderTextWithMarks(textNode)
              end
            else
              html = html .. contentful.renderRichText(itemContent)
            end
          end
          html = html .. "</li>\n"
        end
      end
      html = html .. "</ol>\n"
      
    -- Handle blockquotes
    elseif node.nodeType == "blockquote" and node.content then
      html = html .. "<blockquote>\n"
      for _, quoteContent in ipairs(node.content) do
        if quoteContent.nodeType == "paragraph" and quoteContent.content then
          html = html .. "<p>"
          for _, textNode in ipairs(quoteContent.content) do
            html = html .. contentful.renderTextWithMarks(textNode)
          end
          html = html .. "</p>\n"
        else
          html = html .. contentful.renderRichText(quoteContent)
        end
      end
      html = html .. "</blockquote>\n"
      
    -- Handle horizontal rule
    elseif node.nodeType == "hr" then
      html = html .. "<hr />\n"
      
    -- Handle table (basic support)
    elseif node.nodeType == "table" and node.content then
      html = html .. "<table>\n"
      for _, row in ipairs(node.content) do
        if row.nodeType == "table-row" and row.content then
          html = html .. "<tr>\n"
          for _, cell in ipairs(row.content) do
            if cell.nodeType == "table-cell" or cell.nodeType == "table-header-cell" then
              local tag = cell.nodeType == "table-header-cell" and "th" or "td"
              html = html .. "<" .. tag .. ">"
              if cell.content then
                for _, cellContent in ipairs(cell.content) do
                  html = html .. contentful.renderRichText(cellContent)
                end
              end
              html = html .. "</" .. tag .. ">\n"
            end
          end
          html = html .. "</tr>\n"
        end
      end
      html = html .. "</table>\n"
    end
  end
  return html
end

-- Basic rich text processing for pages (simpler version)
function contentful.renderBasicRichText(richTextObj, includes)
  if not richTextObj or not richTextObj.content then
    return ""
  end
  
  local html = ""
  for _, node in ipairs(richTextObj.content) do
    if node.nodeType == "paragraph" and node.content then
      html = html .. "<p>"
      for _, textNode in ipairs(node.content) do
        if textNode.nodeType == "text" then
          html = html .. (textNode.value or "")
        end
      end
      html = html .. "</p>\n"
    elseif node.nodeType == "heading-1" and node.content then
      html = html .. "<h1>"
      for _, textNode in ipairs(node.content) do
        if textNode.nodeType == "text" then
          html = html .. (textNode.value or "")
        end
      end
      html = html .. "</h1>\n"
    elseif node.nodeType == "heading-2" and node.content then
      html = html .. "<h2>"
      for _, textNode in ipairs(node.content) do
        if textNode.nodeType == "text" then
          html = html .. (textNode.value or "")
        end
      end
      html = html .. "</h2>\n"
    end
  end
  return html
end

return contentful