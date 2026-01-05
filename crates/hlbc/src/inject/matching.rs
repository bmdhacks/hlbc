use std::collections::HashMap;

use crate::types::{Function, Type};
use crate::{Bytecode, Resolve};

/// Match a qualified function name against a pattern with wildcard support.
///
/// Patterns use `.` as segment separator and support:
/// - `*` matches any single segment (e.g., `pkg.*.method` matches `pkg.Foo.method`)
/// - `**` matches zero or more segments (e.g., `pkg.**` matches `pkg.Foo.bar.method`)
/// - Exact match if no wildcards
///
/// Examples:
/// - `h3d.impl.GlDriver.clear` - exact match
/// - `h3d.impl.GlDriver.*` - all methods of GlDriver
/// - `hxsl.*.*` - all classes in hxsl package (one level)
/// - `h3d.impl.**` - all functions in h3d.impl and subpackages
pub fn matches_pattern(name: &str, pattern: &str) -> bool {
    // Skip anonymous closures - they'll be pulled in as dependencies
    if name == "<none>" || name.is_empty() {
        return false;
    }

    let name_parts: Vec<&str> = name.split('.').collect();
    let pattern_parts: Vec<&str> = pattern.split('.').collect();

    matches_parts(&name_parts, &pattern_parts)
}

/// Recursive helper for pattern matching
fn matches_parts(name: &[&str], pattern: &[&str]) -> bool {
    match (name.first(), pattern.first()) {
        // Both exhausted - match!
        (None, None) => true,

        // Pattern exhausted but name has more - no match
        (Some(_), None) => false,

        // Name exhausted but pattern has more - only match if remaining is **
        (None, Some(&"**")) => matches_parts(&[], &pattern[1..]),
        (None, Some(_)) => false,

        // ** matches zero or more segments
        (Some(_), Some(&"**")) => {
            // Try matching ** against zero segments (skip **)
            if matches_parts(name, &pattern[1..]) {
                return true;
            }
            // Try matching ** against one segment (consume one name part)
            matches_parts(&name[1..], pattern)
        }

        // * matches exactly one segment
        (Some(_), Some(&"*")) => matches_parts(&name[1..], &pattern[1..]),

        // Exact segment match
        (Some(n), Some(p)) => {
            if *n == *p {
                matches_parts(&name[1..], &pattern[1..])
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod pattern_tests {
    use super::matches_pattern;

    #[test]
    fn test_exact_match() {
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.impl.GlDriver.clear"));
        assert!(!matches_pattern("h3d.impl.GlDriver.clear", "h3d.impl.GlDriver.draw"));
    }

    #[test]
    fn test_single_wildcard() {
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.impl.GlDriver.*"));
        assert!(matches_pattern("h3d.impl.GlDriver.draw", "h3d.impl.GlDriver.*"));
        assert!(!matches_pattern("h3d.impl.GlDriver.clear", "h3d.impl.Driver.*"));
    }

    #[test]
    fn test_middle_wildcard() {
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.*.GlDriver.clear"));
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.impl.*.clear"));
        assert!(!matches_pattern("h3d.impl.GlDriver.clear", "h3d.*.clear")); // * is one segment
    }

    #[test]
    fn test_double_wildcard() {
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.**"));
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.impl.**"));
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "**"));
        assert!(matches_pattern("hxsl.Linker.link", "hxsl.**"));
    }

    #[test]
    fn test_double_wildcard_middle() {
        assert!(matches_pattern("h3d.impl.GlDriver.clear", "h3d.**.clear"));
        assert!(matches_pattern("h3d.a.b.c.clear", "h3d.**.clear"));
    }

    #[test]
    fn test_anonymous_closure() {
        assert!(!matches_pattern("<none>", "**"));
        assert!(!matches_pattern("<none>", "*"));
        assert!(!matches_pattern("", "**"));
    }

    #[test]
    fn test_package_level() {
        // All classes in package (one level)
        assert!(matches_pattern("hxsl.Linker.link", "hxsl.*.*"));
        assert!(matches_pattern("hxsl.GlslOut.run", "hxsl.*.*"));
        assert!(!matches_pattern("hxsl.sub.Foo.bar", "hxsl.*.*")); // too deep
    }
}

/// A fully qualified function name (type.method or just method for standalone functions)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub type_name: Option<String>,
    pub method_name: String,
}

impl QualifiedName {
    /// Build a qualified name from a function and its bytecode context
    pub fn from_function(code: &Bytecode, func: &Function) -> Self {
        let method_name = code.get(func.name).to_string();
        let type_name = func.parent.and_then(|p| {
            match code.get(p) {
                Type::Obj(obj) | Type::Struct(obj) => Some(code.get(obj.name).to_string()),
                _ => None,
            }
        });
        Self { type_name, method_name }
    }

    /// Get the full name as a string (e.g., "h2d.Scene.render" or just "main")
    pub fn full_name(&self) -> String {
        match &self.type_name {
            Some(t) => format!("{}.{}", t, self.method_name),
            None => self.method_name.clone(),
        }
    }
}

impl std::fmt::Display for QualifiedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_name())
    }
}

/// Index of functions by their qualified names
pub struct FunctionIndex {
    /// Maps qualified name to function pool index
    by_qualified_name: HashMap<String, usize>,
    /// Maps function pool index to qualified name
    by_index: HashMap<usize, String>,
}

impl FunctionIndex {
    /// Build an index from bytecode
    pub fn build(code: &Bytecode) -> Self {
        let mut by_qualified_name = HashMap::new();
        let mut by_index = HashMap::new();

        for (i, func) in code.functions.iter().enumerate() {
            let qname = QualifiedName::from_function(code, func);
            let full = qname.full_name();
            by_qualified_name.insert(full.clone(), i);
            by_index.insert(i, full);
        }

        Self { by_qualified_name, by_index }
    }

    /// Find a function by its qualified name
    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_qualified_name.get(name).copied()
    }

    /// Get the qualified name for a function index
    pub fn get_name(&self, index: usize) -> Option<&str> {
        self.by_index.get(&index).map(|s| s.as_str())
    }

    /// Iterate over all entries
    pub fn iter(&self) -> impl Iterator<Item = (&str, usize)> {
        self.by_qualified_name.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// Get the number of indexed functions
    pub fn len(&self) -> usize {
        self.by_qualified_name.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.by_qualified_name.is_empty()
    }
}
