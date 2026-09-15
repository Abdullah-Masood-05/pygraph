use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::*;

use super::types::TypeRegistry;

#[derive(Clone, Debug)]
pub enum Record {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(u32),
    Bytes(Vec<u8>),
    List(Vec<u32>),
    Tuple(Vec<u32>),
    Dict(Vec<(u32, u32)>),
    Set(Vec<u32>),
    FrozenSet(Vec<u32>),
    Dataclass { type_id: u16, fields: Vec<u32> },
    Reference(u32),
}

#[derive(Clone, Debug, Default)]
pub struct ObjectGraph {
    pub strings: Vec<String>,
    pub records: Vec<Option<Record>>,
    pub string_index: HashMap<String, u32>,
}

impl ObjectGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.string_index.get(s) {
            return idx;
        }
        let idx = self.strings.len() as u32;
        self.strings.push(s.to_string());
        self.string_index.insert(s.to_string(), idx);
        idx
    }

    pub fn push_placeholder(&mut self) -> u32 {
        let id = self.records.len() as u32;
        self.records.push(None);
        id
    }

    pub fn set_record(&mut self, id: u32, record: Record) {
        self.records[id as usize] = Some(record);
    }
}

pub struct Walker<'py> {
    py: Python<'py>,
    memo: HashMap<usize, u32>,
    graph: ObjectGraph,
    type_registry: TypeRegistry,
}

impl<'py> Walker<'py> {
    pub fn new(py: Python<'py>) -> Self {
        Self {
            py,
            memo: HashMap::new(),
            graph: ObjectGraph::new(),
            type_registry: TypeRegistry::new(),
        }
    }

    pub fn walk(&mut self, obj: &Bound<'py, PyAny>) -> PyResult<u32> {
        let ptr = obj.as_ptr() as usize;
        if let Some(&ref_id) = self.memo.get(&ptr) {
            let id = self.graph.push_placeholder();
            self.graph.set_record(id, Record::Reference(ref_id));
            return Ok(id);
        }

        let ob_type = obj.get_type();
        let type_name: String = ob_type.name()?.to_string();

        match type_name.as_str() {
            "NoneType" => {
                let id = self.graph.push_placeholder();
                self.graph.set_record(id, Record::None);
                self.memo.insert(ptr, id);
                Ok(id)
            }
            "bool" => {
                let val: bool = obj.extract()?;
                let id = self.graph.push_placeholder();
                self.graph.set_record(id, Record::Bool(val));
                self.memo.insert(ptr, id);
                Ok(id)
            }
            "int" => {
                let val: i64 = obj.extract()?;
                let id = self.graph.push_placeholder();
                self.graph.set_record(id, Record::Int(val));
                self.memo.insert(ptr, id);
                Ok(id)
            }
            "float" => {
                let val: f64 = obj.extract()?;
                let id = self.graph.push_placeholder();
                self.graph.set_record(id, Record::Float(val));
                self.memo.insert(ptr, id);
                Ok(id)
            }
            "str" => {
                let val: String = obj.extract()?;
                let idx = self.graph.intern_string(&val);
                let id = self.graph.push_placeholder();
                self.graph.set_record(id, Record::String(idx));
                self.memo.insert(ptr, id);
                Ok(id)
            }
            "bytes" => {
                let val: Vec<u8> = obj.extract()?;
                let id = self.graph.push_placeholder();
                self.graph.set_record(id, Record::Bytes(val));
                self.memo.insert(ptr, id);
                Ok(id)
            }
            "list" => {
                let list = obj.downcast::<PyList>()?;
                let id = self.graph.push_placeholder();
                self.memo.insert(ptr, id);

                let mut refs = Vec::with_capacity(list.len());
                for item in list.iter() {
                    refs.push(self.walk(&item)?);
                }
                self.graph.set_record(id, Record::List(refs));
                Ok(id)
            }
            "tuple" => {
                let tup = obj.downcast::<PyTuple>()?;
                let id = self.graph.push_placeholder();
                self.memo.insert(ptr, id);

                let mut refs = Vec::with_capacity(tup.len());
                for item in tup.iter() {
                    refs.push(self.walk(&item)?);
                }
                self.graph.set_record(id, Record::Tuple(refs));
                Ok(id)
            }
            "dict" => {
                let dict = obj.downcast::<PyDict>()?;
                let id = self.graph.push_placeholder();
                self.memo.insert(ptr, id);

                let mut pairs = Vec::with_capacity(dict.len());
                for (k, v) in dict.iter() {
                    let kr = self.walk(&k)?;
                    let vr = self.walk(&v)?;
                    pairs.push((kr, vr));
                }
                self.graph.set_record(id, Record::Dict(pairs));
                Ok(id)
            }
            "set" => {
                let set = obj.downcast::<PySet>()?;
                let id = self.graph.push_placeholder();
                self.memo.insert(ptr, id);

                let mut refs = Vec::with_capacity(set.len());
                for item in set.iter() {
                    refs.push(self.walk(&item)?);
                }
                self.graph.set_record(id, Record::Set(refs));
                Ok(id)
            }
            "frozenset" => {
                let fs = obj.downcast::<PyFrozenSet>()?;
                let id = self.graph.push_placeholder();
                self.memo.insert(ptr, id);

                let mut refs = Vec::with_capacity(fs.len());
                for item in fs.iter() {
                    refs.push(self.walk(&item)?);
                }
                self.graph.set_record(id, Record::FrozenSet(refs));
                Ok(id)
            }
            _ => {
                if is_dataclass(self.py, obj)? {
                    self.walk_dataclass(obj, &type_name, ptr)
                } else {
                    Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "Unsupported type: {}",
                        type_name
                    )))
                }
            }
        }
    }

    fn walk_dataclass(&mut self, obj: &Bound<'py, PyAny>, type_name: &str, ptr: usize) -> PyResult<u32> {
        let fields_dict: Bound<'py, PyDict> = obj.getattr("__dataclass_fields__")?.downcast_into()?;
        let field_names: Vec<String> = fields_dict
            .keys()
            .iter()
            .map(|k| k.extract::<String>())
            .collect::<PyResult<_>>()?;

        let schema_version: u32 = obj
            .getattr("__pysafe_pickle_version__")
            .or_else(|_| obj.getattr("__pygraph_version__"))
            .and_then(|v| v.extract())
            .unwrap_or(0);

        let type_id = self.type_registry.register(
            type_name,
            &field_names.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            schema_version,
        );

        let id = self.graph.push_placeholder();
        self.memo.insert(ptr, id);

        let mut field_refs = Vec::with_capacity(field_names.len());
        for fname in &field_names {
            let val = obj.getattr(fname.as_str())?;
            field_refs.push(self.walk(&val)?);
        }
        self.graph
            .set_record(id, Record::Dataclass { type_id, fields: field_refs });
        Ok(id)
    }

    pub fn into_parts(self) -> (ObjectGraph, TypeRegistry) {
        (self.graph, self.type_registry)
    }
}

fn is_dataclass(py: Python, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let dataclasses = py.import("dataclasses")?;
    let result = dataclasses.getattr("is_dataclass")?.call1((obj,))?;
    result.extract()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_string() {
        let mut graph = ObjectGraph::new();
        let idx1 = graph.intern_string("hello");
        let idx2 = graph.intern_string("hello");
        let idx3 = graph.intern_string("world");
        assert_eq!(idx1, idx2);
        assert_eq!(idx1, 0);
        assert_eq!(idx3, 1);
    }

    #[test]
    fn test_push_and_set() {
        let mut graph = ObjectGraph::new();
        let id = graph.push_placeholder();
        assert!(graph.records[id as usize].is_none());
        graph.set_record(id, Record::None);
        assert!(graph.records[id as usize].is_some());
    }
}
