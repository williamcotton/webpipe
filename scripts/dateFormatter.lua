local dateFormatter = {}

function dateFormatter.formatPublishedDate(publishedDate)
  if not publishedDate or publishedDate == "" then
    return ""
  end
  
  -- Convert ISO date to readable format
  local year, month, day = publishedDate:match("(%d+)-(%d+)-(%d+)")
  if year and month and day then
    local timestamp = os.time({year=tonumber(year), month=tonumber(month), day=tonumber(day)})
    local dayNum = tonumber(os.date("%d", timestamp))
    local suffix = "th"
    if dayNum % 10 == 1 and dayNum ~= 11 then
      suffix = "st"
    elseif dayNum % 10 == 2 and dayNum ~= 12 then
      suffix = "nd"
    elseif dayNum % 10 == 3 and dayNum ~= 13 then
      suffix = "rd"
    end
    return os.date("%B ", timestamp) .. dayNum .. suffix .. os.date(", %Y", timestamp)
  else
    return publishedDate
  end
end

return dateFormatter