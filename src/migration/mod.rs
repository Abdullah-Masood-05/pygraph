use std::collections::HashMap;
use pyo3::prelude::*;

pub struct MigrationEntry {
    pub from_version: u32,
    pub to_version: u32,
    pub func: Py<PyAny>,
}

#[derive(Default)]
pub struct MigrationRegistry {
    pub migrations: HashMap<String, Vec<MigrationEntry>>,
}

impl MigrationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, type_name: &str, from_version: u32, to_version: u32, func: Py<PyAny>) {
        self.migrations
            .entry(type_name.to_string())
            .or_default()
            .push(MigrationEntry {
                from_version,
                to_version,
                func,
            });
    }

    pub fn get_chain(&self, type_name: &str, from: u32, to: u32) -> Option<Vec<&MigrationEntry>> {
        let entries = self.migrations.get(type_name)?;
        if from >= to {
            return Some(Vec::new());
        }

        let mut chain = Vec::new();
        let mut current = from;

        while current < to {
            let next = entries.iter()
                .find(|e| e.from_version == current && e.to_version == current + 1)
                .or_else(|| entries.iter().find(|e| e.from_version == current && e.to_version > current && e.to_version <= to));

            match next {
                Some(entry) => {
                    current = entry.to_version;
                    chain.push(entry);
                }
                None => return None,
            }
        }

        Some(chain)
    }
}
