use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TypeInfo {
    pub name: String,
    pub fields: Vec<String>,
    pub schema_version: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TypeRegistry {
    pub types: Vec<TypeInfo>,
    pub name_to_id: HashMap<String, u16>,
    /// (name, field list) -> id, for O(1) dedup in `register`.
    by_signature: HashMap<(String, Vec<String>), u16>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type. An existing entry is reused only when both the name AND
    /// the field list match, so distinct classes sharing a `__name__` get distinct
    /// ids (`name_to_id` keeps the first id registered under a name). The
    /// serializer applies the same rule, so type-table positions stay aligned.
    pub fn register(&mut self, name: &str, fields: &[&str], schema_version: u32) -> u16 {
        let key = (
            name.to_string(),
            fields.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        );
        if let Some(&id) = self.by_signature.get(&key) {
            return id;
        }
        let id = self.types.len() as u16;
        self.types.push(TypeInfo {
            name: key.0.clone(),
            fields: key.1.clone(),
            schema_version,
        });
        self.name_to_id.entry(key.0.clone()).or_insert(id);
        self.by_signature.insert(key, id);
        id
    }

    pub fn get_id(&self, name: &str) -> Option<u16> {
        self.name_to_id.get(name).copied()
    }

    pub fn get_type(&self, id: u16) -> Option<&TypeInfo> {
        self.types.get(id as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_name_different_fields_get_distinct_ids() {
        let mut reg = TypeRegistry::new();
        let a = reg.register("Dup", &["x", "y"], 0);
        let b = reg.register("Dup", &["a", "b", "c"], 0);
        assert_ne!(a, b);
        assert_eq!(reg.register("Dup", &["x", "y"], 0), a);
        assert_eq!(reg.get_type(b).unwrap().fields, vec!["a", "b", "c"]);
        assert_eq!(reg.get_id("Dup"), Some(a));
    }

    #[test]
    fn test_register_and_lookup() {
        let mut reg = TypeRegistry::new();
        let id = reg.register("MyClass", &["x", "y"], 1);
        assert_eq!(id, 0);
        assert_eq!(reg.get_id("MyClass"), Some(0));
        let ti = reg.get_type(0).unwrap();
        assert_eq!(ti.fields, vec!["x", "y"]);
        assert_eq!(ti.schema_version, 1);
    }

    #[test]
    fn test_same_name_returns_same_id() {
        let mut reg = TypeRegistry::new();
        let id1 = reg.register("Foo", &["a"], 1);
        let id2 = reg.register("Foo", &["a"], 2);
        assert_eq!(id1, id2);
    }
}
