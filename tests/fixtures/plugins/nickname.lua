-- genechart parse plugin (plugins.parse.indi)
--
-- For a restricted set of individual ids, append the GEDCOM NICK nickname to the
-- given name, in double quotes — the customary real-life form, e.g.
--   Robert "Bob" Smith
--
-- NICK is a sub-tag of NAME (`2 NICK ...`) that genechart does not model, so it
-- arrives via the `unparsed` array rather than a named field.

local targets = { I1 = true } -- limit the rewrite to these individual ids

function on_individual(ind)
  if not targets[ind.id] or not ind.given then
    return
  end
  for _, u in ipairs(ind.unparsed) do
    if u.tag == "NICK" and u.value ~= "" then
      return { given = ind.given .. ' "' .. u.value .. '"' }
    end
  end
end
