use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TypeInfo {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TypeRegistry {
    pub types: Vec<TypeInfo>,
    pub name_to_id: HashMap<String, u16>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: &str, fields: &[&str]) -> u16 {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = self.types.len() as u16;
        self.types.push(TypeInfo {
            name: name.to_string(),
            fields: fields.iter().map(|s| s.to_string()).collect(),
        });
        self.name_to_id.insert(name.to_string(), id);
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
    fn test_register_and_lookup() {
        let mut reg = TypeRegistry::new();
        let id = reg.register("MyClass", &["x", "y"]);
        assert_eq!(id, 0);
        assert_eq!(reg.get_id("MyClass"), Some(0));
        assert_eq!(reg.get_type(0).unwrap().fields, vec!["x", "y"]);
    }

    #[test]
    fn test_same_name_returns_same_id() {
        let mut reg = TypeRegistry::new();
        let id1 = reg.register("Foo", &["a"]);
        let id2 = reg.register("Foo", &["a"]);
        assert_eq!(id1, id2);
    }
}
