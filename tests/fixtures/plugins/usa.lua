-- genechart parse plugin (plugins.parse.all)
--
-- Append ", USA" to any birth/death/marriage place that ends with a US two-letter
-- state abbreviation, e.g. "Boston, MA" -> "Boston, MA, USA".
--
-- An `all` script provides BOTH callbacks; it runs before any type-specific
-- (plugins.parse.indi / plugins.parse.fam) script.

local STATES = {
  AL = 1, AK = 1, AZ = 1, AR = 1, CA = 1, CO = 1, CT = 1, DE = 1, FL = 1, GA = 1,
  HI = 1, ID = 1, IL = 1, IN = 1, IA = 1, KS = 1, KY = 1, LA = 1, ME = 1, MD = 1,
  MA = 1, MI = 1, MN = 1, MS = 1, MO = 1, MT = 1, NE = 1, NV = 1, NH = 1, NJ = 1,
  NM = 1, NY = 1, NC = 1, ND = 1, OH = 1, OK = 1, OR = 1, PA = 1, RI = 1, SC = 1,
  SD = 1, TN = 1, TX = 1, UT = 1, VT = 1, VA = 1, WA = 1, WV = 1, WI = 1, WY = 1,
  DC = 1,
}

local function fixed(place)
  if not place then
    return nil
  end
  local code = place:match(",%s*(%u%u)$")
  if code and STATES[code] then
    return place .. ", USA"
  end
  return nil
end

function on_individual(ind)
  local changes = {}
  if ind.birth then
    local p = fixed(ind.birth.place)
    if p then changes.birth = { place = p } end
  end
  if ind.death then
    local p = fixed(ind.death.place)
    if p then changes.death = { place = p } end
  end
  if next(changes) then
    return changes
  end
end

function on_family(fam)
  if fam.marriage then
    local p = fixed(fam.marriage.place)
    if p then
      return { marriage = { place = p } }
    end
  end
end
