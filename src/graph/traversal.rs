use rustc_hash::FxHashMap;

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
    pub string_index: FxHashMap<String, u32>,
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
        let owned = s.to_string();
        self.string_index.insert(owned.clone(), idx);
        self.strings.push(owned);
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

enum WalkItem<'py> {
    Walk {
        obj: Bound<'py, PyAny>,
        depth: usize,
    },
    FinishList {
        id: u32,
        child_count: usize,
    },
    FinishTuple {
        id: u32,
        child_count: usize,
    },
    FinishDict {
        id: u32,
        pair_count: usize,
    },
    FinishSet {
        id: u32,
        child_count: usize,
    },
    FinishFrozenSet {
        id: u32,
        child_count: usize,
    },
    FinishDataclass {
        id: u32,
        type_id: u16,
        field_count: usize,
    },
}

pub struct Walker<'py> {
    _phantom: std::marker::PhantomData<&'py ()>,
    memo: FxHashMap<usize, u32>,
    graph: ObjectGraph,
    type_registry: TypeRegistry,
    pinned_objects: Vec<Py<PyAny>>,
    pinned_types: Vec<Py<PyType>>,
    dataclass_cache: FxHashMap<usize, Option<(u16, Vec<String>)>>,
    max_depth: usize,
}

impl<'py> Walker<'py> {
    pub fn new(py: Python<'py>) -> Self {
        let max_depth: usize = py
            .import("sys")
            .and_then(|sys| sys.getattr("getrecursionlimit"))
            .and_then(|func| func.call0())
            .and_then(|res| res.extract())
            .unwrap_or(1000);

        Self {
            _phantom: std::marker::PhantomData,
            memo: FxHashMap::default(),
            graph: ObjectGraph::new(),
            type_registry: TypeRegistry::new(),
            pinned_objects: Vec::new(),
            pinned_types: Vec::new(),
            dataclass_cache: FxHashMap::default(),
            max_depth,
        }
    }

    pub fn walk(&mut self, root: &Bound<'py, PyAny>) -> PyResult<u32> {
        let mut eval_stack: Vec<u32> = Vec::new();
        let mut work_stack: Vec<WalkItem<'py>> = vec![WalkItem::Walk {
            obj: root.clone(),
            depth: 0,
        }];

        while let Some(item) = work_stack.pop() {
            match item {
                WalkItem::Walk { obj, depth } => {
                    if depth >= self.max_depth {
                        return Err(pyo3::exceptions::PyRecursionError::new_err(
                            "maximum recursion depth exceeded in serialization",
                        ));
                    }

                    // Tier 1: Skip memoizing immutable scalars completely
                    if obj.is_none() {
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::None);
                        eval_stack.push(id);
                        continue;
                    }
                    if obj.is_exact_instance_of::<PyBool>() {
                        let val: bool = obj.extract()?;
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::Bool(val));
                        eval_stack.push(id);
                        continue;
                    }
                    if obj.is_exact_instance_of::<PyInt>() {
                        let val: i64 = obj.extract()?;
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::Int(val));
                        eval_stack.push(id);
                        continue;
                    }
                    if obj.is_exact_instance_of::<PyFloat>() {
                        let val: f64 = obj.extract()?;
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::Float(val));
                        eval_stack.push(id);
                        continue;
                    }

                    // Non-scalar objects: check memo
                    let ptr = obj.as_ptr() as usize;
                    if let Some(&ref_id) = self.memo.get(&ptr) {
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::Reference(ref_id));
                        eval_stack.push(id);
                        continue;
                    }

                    if obj.is_exact_instance_of::<PyString>() {
                        let val: &str = obj.extract()?;
                        let idx = self.graph.intern_string(val);
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::String(idx));
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());
                        eval_stack.push(id);
                    } else if obj.is_exact_instance_of::<PyList>() {
                        let list = obj.downcast::<PyList>()?;
                        let id = self.graph.push_placeholder();
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());

                        let len = list.len();
                        work_stack.push(WalkItem::FinishList {
                            id,
                            child_count: len,
                        });
                        for i in (0..len).rev() {
                            let item = list.get_item(i)?;
                            work_stack.push(WalkItem::Walk {
                                obj: item,
                                depth: depth + 1,
                            });
                        }
                    } else if obj.is_exact_instance_of::<PyDict>() {
                        let dict = obj.downcast::<PyDict>()?;
                        let id = self.graph.push_placeholder();
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());

                        let items: Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)> =
                            dict.iter().collect();
                        let count = items.len();
                        work_stack.push(WalkItem::FinishDict {
                            id,
                            pair_count: count,
                        });
                        for (k, v) in items.into_iter().rev() {
                            work_stack.push(WalkItem::Walk {
                                obj: v,
                                depth: depth + 1,
                            });
                            work_stack.push(WalkItem::Walk {
                                obj: k,
                                depth: depth + 1,
                            });
                        }
                    } else if obj.is_exact_instance_of::<PyTuple>() {
                        let tup = obj.downcast::<PyTuple>()?;
                        let id = self.graph.push_placeholder();
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());

                        let len = tup.len();
                        work_stack.push(WalkItem::FinishTuple {
                            id,
                            child_count: len,
                        });
                        for i in (0..len).rev() {
                            let item = tup.get_item(i)?;
                            work_stack.push(WalkItem::Walk {
                                obj: item,
                                depth: depth + 1,
                            });
                        }
                    } else if obj.is_exact_instance_of::<PyBytes>() {
                        let val = obj.downcast::<PyBytes>()?;
                        let id = self.graph.push_placeholder();
                        self.graph.set_record(id, Record::Bytes(val.as_bytes().to_vec()));
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());
                        eval_stack.push(id);
                    } else if obj.is_exact_instance_of::<PySet>() {
                        let set = obj.downcast::<PySet>()?;
                        let id = self.graph.push_placeholder();
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());

                        let items: Vec<Bound<'py, PyAny>> = set.iter().collect();
                        let count = items.len();
                        work_stack.push(WalkItem::FinishSet {
                            id,
                            child_count: count,
                        });
                        for item in items.into_iter().rev() {
                            work_stack.push(WalkItem::Walk {
                                obj: item,
                                depth: depth + 1,
                            });
                        }
                    } else if obj.is_exact_instance_of::<PyFrozenSet>() {
                        let fs = obj.downcast::<PyFrozenSet>()?;
                        let id = self.graph.push_placeholder();
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());

                        let items: Vec<Bound<'py, PyAny>> = fs.iter().collect();
                        let count = items.len();
                        work_stack.push(WalkItem::FinishFrozenSet {
                            id,
                            child_count: count,
                        });
                        for item in items.into_iter().rev() {
                            work_stack.push(WalkItem::Walk {
                                obj: item,
                                depth: depth + 1,
                            });
                        }
                    } else {
                        // Non-builtin object: inspect class for dataclass support
                        let ob_type = obj.get_type();
                        let type_ptr = ob_type.as_ptr() as usize;

                        let cached = self.dataclass_cache.get(&type_ptr).cloned();
                        let (type_id, field_names) = match cached {
                            Some(Some(info)) => info,
                            Some(None) => {
                                let type_name: String = ob_type.name()?.to_string();
                                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                                    "Unsupported type: {}",
                                    type_name
                                )));
                            }
                            None => {
                                if ob_type.hasattr("__dataclass_fields__")? {
                                    self.pinned_types.push(ob_type.clone().unbind());
                                    let type_name: String = ob_type.name()?.to_string();
                                    let fields_dict: Bound<'py, PyDict> =
                                        obj.getattr("__dataclass_fields__")?.downcast_into()?;
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
                                        &type_name,
                                        &field_names.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                                        schema_version,
                                    );
                                    self.dataclass_cache
                                        .insert(type_ptr, Some((type_id, field_names.clone())));
                                    (type_id, field_names)
                                } else {
                                    self.pinned_types.push(ob_type.clone().unbind());
                                    self.dataclass_cache.insert(type_ptr, None);
                                    let type_name: String = ob_type.name()?.to_string();
                                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                                        "Unsupported type: {}",
                                        type_name
                                    )));
                                }
                            }
                        };

                        let id = self.graph.push_placeholder();
                        self.memo.insert(ptr, id);
                        self.pinned_objects.push(obj.clone().unbind());

                        work_stack.push(WalkItem::FinishDataclass {
                            id,
                            type_id,
                            field_count: field_names.len(),
                        });
                        for fname in field_names.iter().rev() {
                            let val = obj.getattr(fname.as_str())?;
                            work_stack.push(WalkItem::Walk {
                                obj: val,
                                depth: depth + 1,
                            });
                        }
                    }
                }
                WalkItem::FinishList { id, child_count } => {
                    let start = eval_stack.len() - child_count;
                    let refs: Vec<u32> = eval_stack.drain(start..).collect();
                    self.graph.set_record(id, Record::List(refs));
                    eval_stack.push(id);
                }
                WalkItem::FinishTuple { id, child_count } => {
                    let start = eval_stack.len() - child_count;
                    let refs: Vec<u32> = eval_stack.drain(start..).collect();
                    self.graph.set_record(id, Record::Tuple(refs));
                    eval_stack.push(id);
                }
                WalkItem::FinishDict { id, pair_count } => {
                    let start = eval_stack.len() - (2 * pair_count);
                    let kv: Vec<u32> = eval_stack.drain(start..).collect();
                    let pairs: Vec<(u32, u32)> = kv
                        .chunks_exact(2)
                        .map(|chunk| (chunk[0], chunk[1]))
                        .collect();
                    self.graph.set_record(id, Record::Dict(pairs));
                    eval_stack.push(id);
                }
                WalkItem::FinishSet { id, child_count } => {
                    let start = eval_stack.len() - child_count;
                    let refs: Vec<u32> = eval_stack.drain(start..).collect();
                    self.graph.set_record(id, Record::Set(refs));
                    eval_stack.push(id);
                }
                WalkItem::FinishFrozenSet { id, child_count } => {
                    let start = eval_stack.len() - child_count;
                    let refs: Vec<u32> = eval_stack.drain(start..).collect();
                    self.graph.set_record(id, Record::FrozenSet(refs));
                    eval_stack.push(id);
                }
                WalkItem::FinishDataclass {
                    id,
                    type_id,
                    field_count,
                } => {
                    let start = eval_stack.len() - field_count;
                    let fields: Vec<u32> = eval_stack.drain(start..).collect();
                    self.graph
                        .set_record(id, Record::Dataclass { type_id, fields });
                    eval_stack.push(id);
                }
            }
        }

        eval_stack.pop().ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Traversal produced no root record")
        })
    }

    pub fn into_parts(self) -> (ObjectGraph, TypeRegistry) {
        (self.graph, self.type_registry)
    }
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
