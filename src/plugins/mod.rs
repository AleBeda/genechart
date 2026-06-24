//! Experimental plugin system.
//!
//! Currently provides **parse-time** Lua hooks: after the parser builds each
//! `Individual`/`Family`, configured Lua scripts may inspect it (including the
//! GEDCOM tags the parser did not map to a field) and return modified text/event
//! attributes, which are committed to the internal representation.
//!
//! The whole Lua engine is gated behind the optional `lua` Cargo feature so a
//! default build needs no C compiler. `PluginEngine` exists in both builds (a
//! no-op stub without the feature) so the parser stays free of `cfg`.
//!
//! Design notes live under "Plugin system" in CLAUDE.md.

// These are only referenced by the stub (no-`lua`) impl; the Lua impl imports
// its own copies inside `lua_impl`.
#[cfg(not(feature = "lua"))]
use crate::parser::genrep::{Family, Individual};
#[cfg(not(feature = "lua"))]
use crate::preferences::ParsePluginsPrefs;

/// A GEDCOM line the parser did not map to a struct field (e.g. `2 NICK Jack`).
/// Passed to plugins as the `unparsed` array so scripts can use tags genechart
/// does not model. Fields are only read in `lua`-feature builds.
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "lua"), allow(dead_code))]
pub struct UnparsedTag {
    pub level: u32,
    pub tag: String,
    pub value: String,
}

// ── Stub implementation (feature `lua` disabled) ───────────────────────────────

#[cfg(not(feature = "lua"))]
#[derive(Debug)]
pub struct PluginEngine;

#[cfg(not(feature = "lua"))]
impl PluginEngine {
    /// Without Lua support, configuring any parse plugin is a hard error so the
    /// user is not silently ignored.
    pub fn from_prefs(
        p: &ParsePluginsPrefs,
        _diag: &crate::preferences::DiagnosticsPrefs,
    ) -> anyhow::Result<Self> {
        if !p.indi.is_empty() || !p.fam.is_empty() || !p.all.is_empty() {
            anyhow::bail!(
                "plugins are configured (plugins.parse.*) but this build has no Lua support; \
                 rebuild with `--features lua`"
            );
        }
        Ok(PluginEngine)
    }

    /// An engine with no plugins (used by tests and the merge path).
    #[allow(dead_code)]
    pub fn disabled() -> Self {
        PluginEngine
    }

    /// No scripts are ever active, so the parser skips unparsed-tag capture.
    pub fn active(&self) -> bool {
        false
    }
    pub fn run_individual(&self, _ind: &mut Individual<()>, _unparsed: &[UnparsedTag]) {}
    pub fn run_family(&self, _fam: &mut Family<()>, _unparsed: &[UnparsedTag]) {}
}

#[cfg(all(test, not(feature = "lua")))]
mod stub_tests {
    use super::*;
    use crate::preferences::{DiagnosticsPrefs, ParsePluginsPrefs};

    #[test]
    fn configuring_a_plugin_without_lua_feature_errors() {
        let p = ParsePluginsPrefs {
            indi: "x.lua".into(),
            ..Default::default()
        };
        let err = PluginEngine::from_prefs(&p, &DiagnosticsPrefs::default()).unwrap_err();
        assert!(
            format!("{err}").contains("--features lua"),
            "error should suggest rebuilding: {err}"
        );
    }

    #[test]
    fn no_plugins_configured_is_ok() {
        let p = ParsePluginsPrefs::default();
        assert!(PluginEngine::from_prefs(&p, &DiagnosticsPrefs::default()).is_ok());
    }
}

// ── Lua implementation (feature `lua` enabled) ─────────────────────────────────

#[cfg(feature = "lua")]
pub use lua_impl::PluginEngine;

#[cfg(feature = "lua")]
mod lua_impl {
    use super::UnparsedTag;
    use crate::parser::genrep::{Event, Family, GedDate, Individual};
    use crate::preferences::{DiagnosticsPrefs, ParsePluginsPrefs};
    use anyhow::Context;
    use mlua::{Lua, Table, Value};

    /// One loaded Lua script and which callbacks it defines.
    struct LuaScript {
        lua: Lua,
        has_individual: bool,
        has_family: bool,
        label: String, // "all" | "indi" | "fam", for diagnostics
    }

    impl LuaScript {
        fn load(label: &str, path: &str) -> anyhow::Result<Self> {
            let src = std::fs::read_to_string(path)
                .with_context(|| format!("plugin [{label}]: cannot read script '{path}'"))?;
            let lua = Lua::new();
            lua.load(&src)
                .set_name(path)
                .exec()
                .map_err(|e| anyhow::anyhow!("plugin [{label}]: error loading '{path}': {e}"))?;
            let globals = lua.globals();
            let is_fn = |k: &str| matches!(globals.get::<Value>(k), Ok(Value::Function(_)));
            let has_individual = is_fn("on_individual");
            let has_family = is_fn("on_family");
            Ok(LuaScript {
                lua,
                has_individual,
                has_family,
                label: label.to_string(),
            })
        }
    }

    pub struct PluginEngine {
        all: Option<LuaScript>,
        indi: Option<LuaScript>,
        fam: Option<LuaScript>,
        warnings: bool,
        debug: bool,
    }

    impl PluginEngine {
        pub fn from_prefs(p: &ParsePluginsPrefs, diag: &DiagnosticsPrefs) -> anyhow::Result<Self> {
            let load = |label: &str, path: &str| -> anyhow::Result<Option<LuaScript>> {
                if path.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(LuaScript::load(label, path)?))
                }
            };
            Ok(PluginEngine {
                all: load("all", &p.all)?,
                indi: load("indi", &p.indi)?,
                fam: load("fam", &p.fam)?,
                warnings: diag.warnings,
                debug: diag.debug,
            })
        }

        /// An engine with no plugins (used by tests and the merge path).
        #[allow(dead_code)]
        pub fn disabled() -> Self {
            PluginEngine {
                all: None,
                indi: None,
                fam: None,
                warnings: false,
                debug: false,
            }
        }

        pub fn active(&self) -> bool {
            self.all.is_some() || self.indi.is_some() || self.fam.is_some()
        }

        pub fn run_individual(&self, ind: &mut Individual<()>, unparsed: &[UnparsedTag]) {
            // `all` first, then the type-specific script; each sees prior edits.
            for s in [self.all.as_ref(), self.indi.as_ref()]
                .into_iter()
                .flatten()
            {
                if s.has_individual {
                    self.call_individual(s, ind, unparsed);
                }
            }
        }

        pub fn run_family(&self, fam: &mut Family<()>, unparsed: &[UnparsedTag]) {
            for s in [self.all.as_ref(), self.fam.as_ref()].into_iter().flatten() {
                if s.has_family {
                    self.call_family(s, fam, unparsed);
                }
            }
        }

        fn call_individual(
            &self,
            s: &LuaScript,
            ind: &mut Individual<()>,
            unparsed: &[UnparsedTag],
        ) {
            let res = (|| -> mlua::Result<Value> {
                let tbl = build_individual_table(&s.lua, ind, unparsed)?;
                let func: mlua::Function = s.lua.globals().get("on_individual")?;
                func.call(tbl)
            })();
            match res {
                Ok(Value::Table(changes)) => self.apply_individual(s, ind, &changes),
                Ok(Value::Nil) => {}
                Ok(_) => self.warn(s, &ind.id, "on_individual must return a table or nil"),
                Err(e) => self.warn_runtime(s, "on_individual", &ind.id, &e),
            }
        }

        fn call_family(&self, s: &LuaScript, fam: &mut Family<()>, unparsed: &[UnparsedTag]) {
            let res = (|| -> mlua::Result<Value> {
                let tbl = build_family_table(&s.lua, fam, unparsed)?;
                let func: mlua::Function = s.lua.globals().get("on_family")?;
                func.call(tbl)
            })();
            match res {
                Ok(Value::Table(changes)) => self.apply_family(s, fam, &changes),
                Ok(Value::Nil) => {}
                Ok(_) => self.warn(s, &fam.id, "on_family must return a table or nil"),
                Err(e) => self.warn_runtime(s, "on_family", &fam.id, &e),
            }
        }

        // ── change application (whitelisted, with plausibility checks) ─────────

        fn apply_individual(&self, s: &LuaScript, ind: &mut Individual<()>, ch: &Table) {
            match str_field(ch, "given") {
                StrField::Set(v) => ind.given = Some(v),
                StrField::Wrong => self.warn(s, &ind.id, "given: expected string; ignored"),
                StrField::Absent => {}
            }
            match str_field(ch, "surname") {
                StrField::Set(v) => ind.surname = Some(v),
                StrField::Wrong => self.warn(s, &ind.id, "surname: expected string; ignored"),
                StrField::Absent => {}
            }
            match str_field(ch, "alt_name") {
                StrField::Set(v) => ind.alt_name = Some(v),
                StrField::Wrong => self.warn(s, &ind.id, "alt_name: expected string; ignored"),
                StrField::Absent => {}
            }
            match str_field(ch, "relig_name") {
                StrField::Set(v) => ind.relig_name = Some(v),
                StrField::Wrong => self.warn(s, &ind.id, "relig_name: expected string; ignored"),
                StrField::Absent => {}
            }
            match str_field(ch, "sex") {
                StrField::Set(v) if v.chars().count() == 1 => ind.sex = v.chars().next(),
                StrField::Absent => {}
                _ => self.warn(s, &ind.id, "sex: expected a 1-character string; ignored"),
            }
            self.apply_living(s, &ind.id, ch, &mut ind.living);
            self.apply_notes(s, &ind.id, ch, &mut ind.notes);
            self.apply_event(s, &ind.id, ch, "birth", &mut ind.birth);
            self.apply_event(s, &ind.id, ch, "death", &mut ind.death);
            self.warn_readonly(s, &ind.id, ch, &["id", "fams", "famc", "birth_date"]);
        }

        fn apply_family(&self, s: &LuaScript, fam: &mut Family<()>, ch: &Table) {
            match str_field(ch, "relig_marr") {
                StrField::Set(v) => fam.relig_marr = Some(v),
                StrField::Wrong => self.warn(s, &fam.id, "relig_marr: expected string; ignored"),
                StrField::Absent => {}
            }
            self.apply_notes(s, &fam.id, ch, &mut fam.notes);
            self.apply_event(s, &fam.id, ch, "marriage", &mut fam.marriage);
            self.warn_readonly(s, &fam.id, ch, &["id", "husband", "wife", "children"]);
        }

        fn apply_living(&self, s: &LuaScript, id: &str, ch: &Table, slot: &mut Option<bool>) {
            match ch.get::<Value>("living") {
                Ok(Value::Boolean(b)) => *slot = Some(b),
                Ok(Value::Nil) | Err(_) => {}
                Ok(_) => self.warn(s, id, "living: expected boolean; ignored"),
            }
        }

        fn apply_notes(&self, s: &LuaScript, id: &str, ch: &Table, slot: &mut Vec<String>) {
            match ch.get::<Value>("notes") {
                Ok(Value::Table(t)) => {
                    let mut v = Vec::new();
                    for item in t.sequence_values::<String>() {
                        match item {
                            Ok(line) => v.push(line),
                            Err(_) => self.warn(s, id, "notes: entries must be strings; skipped"),
                        }
                    }
                    *slot = v;
                }
                Ok(Value::Nil) | Err(_) => {}
                Ok(_) => self.warn(s, id, "notes: expected an array of strings; ignored"),
            }
        }

        fn apply_event(
            &self,
            s: &LuaScript,
            id: &str,
            ch: &Table,
            key: &str,
            slot: &mut Option<Event>,
        ) {
            match ch.get::<Value>(key) {
                Ok(Value::Table(t)) => {
                    let e = slot.get_or_insert(Event {
                        date: None,
                        place: None,
                    });
                    match str_field(&t, "date") {
                        StrField::Set(v) => e.date = Some(GedDate { raw: v }),
                        StrField::Wrong => {
                            self.warn(s, id, &format!("{key}.date: expected string; ignored"))
                        }
                        StrField::Absent => {}
                    }
                    match str_field(&t, "place") {
                        StrField::Set(v) => e.place = Some(v),
                        StrField::Wrong => {
                            self.warn(s, id, &format!("{key}.place: expected string; ignored"))
                        }
                        StrField::Absent => {}
                    }
                }
                Ok(Value::Nil) | Err(_) => {}
                Ok(_) => self.warn(s, id, &format!("{key}: expected a {{date=,place=}} table")),
            }
        }

        fn warn_readonly(&self, s: &LuaScript, id: &str, ch: &Table, keys: &[&str]) {
            for k in keys {
                if !matches!(ch.get::<Value>(*k), Ok(Value::Nil) | Err(_)) {
                    self.warn(s, id, &format!("'{k}' is read-only; ignored"));
                }
            }
        }

        // ── diagnostics ────────────────────────────────────────────────────────

        fn warn(&self, s: &LuaScript, id: &str, msg: &str) {
            if self.warnings {
                eprintln!("Warning: plugin [{}] {id}: {msg}", s.label);
            }
        }

        /// Runtime errors are always reported (a broken script needs surfacing),
        /// independent of the diagnostics.warnings gate.
        fn warn_runtime(&self, s: &LuaScript, func: &str, id: &str, e: &mlua::Error) {
            eprintln!(
                "Warning: plugin [{}] {func}({id}) runtime error (record left unchanged): {e}",
                s.label
            );
            let _ = self.debug; // reserved for future verbose tracing
        }
    }

    // ── marshalling: record → Lua table ───────────────────────────────────────

    fn set_opt_str(t: &Table, key: &str, val: &Option<String>) -> mlua::Result<()> {
        if let Some(v) = val {
            t.set(key, v.clone())?;
        }
        Ok(())
    }

    fn str_array(lua: &Lua, items: &[String]) -> mlua::Result<Table> {
        let arr = lua.create_table()?;
        for (i, v) in items.iter().enumerate() {
            arr.set(i + 1, v.clone())?;
        }
        Ok(arr)
    }

    fn event_table(lua: &Lua, e: &Event) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        if let Some(d) = &e.date {
            t.set("date", d.raw.clone())?;
        }
        if let Some(p) = &e.place {
            t.set("place", p.clone())?;
        }
        Ok(t)
    }

    fn unparsed_array(lua: &Lua, unparsed: &[UnparsedTag]) -> mlua::Result<Table> {
        let arr = lua.create_table()?;
        for (i, u) in unparsed.iter().enumerate() {
            let e = lua.create_table()?;
            e.set("level", u.level)?;
            e.set("tag", u.tag.clone())?;
            e.set("value", u.value.clone())?;
            arr.set(i + 1, e)?;
        }
        Ok(arr)
    }

    fn build_individual_table(
        lua: &Lua,
        ind: &Individual<()>,
        unparsed: &[UnparsedTag],
    ) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        t.set("id", ind.id.clone())?;
        set_opt_str(&t, "given", &ind.given)?;
        set_opt_str(&t, "surname", &ind.surname)?;
        if let Some(s) = ind.sex {
            t.set("sex", s.to_string())?;
        }
        if let Some(b) = ind.living {
            t.set("living", b)?;
        }
        set_opt_str(&t, "alt_name", &ind.alt_name)?;
        set_opt_str(&t, "relig_name", &ind.relig_name)?;
        t.set("notes", str_array(lua, &ind.notes)?)?;
        t.set("fams", str_array(lua, &ind.fams)?)?;
        t.set("famc", str_array(lua, &ind.famc)?)?;
        if let Some(e) = &ind.birth {
            t.set("birth", event_table(lua, e)?)?;
        }
        if let Some(e) = &ind.death {
            t.set("death", event_table(lua, e)?)?;
        }
        t.set("unparsed", unparsed_array(lua, unparsed)?)?;
        Ok(t)
    }

    fn build_family_table(
        lua: &Lua,
        fam: &Family<()>,
        unparsed: &[UnparsedTag],
    ) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        t.set("id", fam.id.clone())?;
        set_opt_str(&t, "husband", &fam.husband_id)?;
        set_opt_str(&t, "wife", &fam.wife_id)?;
        t.set("children", str_array(lua, &fam.children_ids)?)?;
        if let Some(e) = &fam.marriage {
            t.set("marriage", event_table(lua, e)?)?;
        }
        set_opt_str(&t, "relig_marr", &fam.relig_marr)?;
        t.set("notes", str_array(lua, &fam.notes)?)?;
        t.set("unparsed", unparsed_array(lua, unparsed)?)?;
        Ok(t)
    }

    // ── reading a string field from the returned change table ──────────────────

    enum StrField {
        Absent,
        Wrong,
        Set(String),
    }

    fn str_field(t: &Table, key: &str) -> StrField {
        match t.get::<Value>(key) {
            Ok(Value::String(s)) => StrField::Set(s.to_string_lossy().to_string()),
            Ok(Value::Nil) | Err(_) => StrField::Absent,
            Ok(_) => StrField::Wrong,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::UnparsedTag;
        use super::*;
        use crate::parser::genrep::{Event, Individual};

        fn engine_from_src(all: &str, indi: &str, fam: &str) -> PluginEngine {
            // Build an engine directly from inline scripts (bypass file IO).
            let mk = |label: &str, src: &str| -> Option<LuaScript> {
                if src.is_empty() {
                    return None;
                }
                let lua = Lua::new();
                lua.load(src).exec().unwrap();
                let g = lua.globals();
                let is_fn = |k: &str| matches!(g.get::<Value>(k), Ok(Value::Function(_)));
                let (hi, hf) = (is_fn("on_individual"), is_fn("on_family"));
                drop(g);
                Some(LuaScript {
                    lua,
                    has_individual: hi,
                    has_family: hf,
                    label: label.to_string(),
                })
            };
            PluginEngine {
                all: mk("all", all),
                indi: mk("indi", indi),
                fam: mk("fam", fam),
                warnings: true,
                debug: false,
            }
        }

        fn sample_indi() -> Individual<()> {
            Individual {
                id: "I1".into(),
                given: Some("Robert".into()),
                surname: Some("Smith".into()),
                sex: Some('M'),
                birth: Some(Event {
                    date: None,
                    place: Some("Boston, MA".into()),
                }),
                death: None,
                fams: vec!["F1".into()],
                famc: vec![],
                alt_name: None,
                relig_name: None,
                living: None,
                nickname: None,
                notes: vec![],
                in_scope: false,
                geo: None,
            }
        }

        #[test]
        fn nickname_rewrites_given_name() {
            let eng = engine_from_src(
                "",
                r#"
                function on_individual(ind)
                  for _, u in ipairs(ind.unparsed) do
                    if u.tag == "NICK" then
                      return { given = ind.given .. ' "' .. u.value .. '"' }
                    end
                  end
                end
                "#,
                "",
            );
            let mut ind = sample_indi();
            let unparsed = vec![UnparsedTag {
                level: 2,
                tag: "NICK".into(),
                value: "Bob".into(),
            }];
            eng.run_individual(&mut ind, &unparsed);
            assert_eq!(ind.given.as_deref(), Some("Robert \"Bob\""));
        }

        #[test]
        fn usa_appends_to_place() {
            let eng = engine_from_src(
                r#"
                function on_individual(ind)
                  if ind.birth and ind.birth.place and ind.birth.place:match(", %u%u$") then
                    return { birth = { place = ind.birth.place .. ", USA" } }
                  end
                end
                "#,
                "",
                "",
            );
            let mut ind = sample_indi();
            eng.run_individual(&mut ind, &[]);
            assert_eq!(
                ind.birth.as_ref().and_then(|e| e.place.as_deref()),
                Some("Boston, MA, USA")
            );
        }

        #[test]
        fn structural_field_is_read_only() {
            let eng = engine_from_src(
                "",
                r#"function on_individual(ind) return { id = "HACKED", fams = {} } end"#,
                "",
            );
            let mut ind = sample_indi();
            eng.run_individual(&mut ind, &[]);
            assert_eq!(ind.id, "I1"); // unchanged
            assert_eq!(ind.fams, vec!["F1".to_string()]); // unchanged
        }

        #[test]
        fn runtime_error_leaves_record_unchanged() {
            let eng = engine_from_src("", r#"function on_individual(ind) error("boom") end"#, "");
            let mut ind = sample_indi();
            eng.run_individual(&mut ind, &[]);
            assert_eq!(ind.given.as_deref(), Some("Robert")); // unchanged
        }

        #[test]
        fn all_runs_before_specific() {
            // `all` sets given="A"; `indi` appends "B" → final "AB" proves order.
            let eng = engine_from_src(
                r#"function on_individual(ind) return { given = "A" } end"#,
                r#"function on_individual(ind) return { given = ind.given .. "B" } end"#,
                "",
            );
            let mut ind = sample_indi();
            eng.run_individual(&mut ind, &[]);
            assert_eq!(ind.given.as_deref(), Some("AB"));
        }
    }
}
