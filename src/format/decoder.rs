use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::*;
use pyo3::BoundObject;

use super::*;
use crate::graph::traversal::Record;
use crate::graph::types::TypeRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooShort(&'static str),
    InvalidMagic,
    InvalidUtf8(String),
    Truncated(&'static str),
    CountExceedsData {
        item: &'static str,
        count: u32,
        max_possible: usize,
    },
    UnknownTag(u8),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort(msg) => write!(f, "Invalid data: too short for {}", msg),
            DecodeError::InvalidMagic => write!(f, "Invalid magic bytes"),
            DecodeError::InvalidUtf8(err) => write!(f, "Invalid UTF-8 in string table: {}", err),
            DecodeError::Truncated(msg) => write!(f, "Invalid data: {} truncated", msg),
            DecodeError::CountExceedsData {
                item,
                count,
                max_possible,
            } => {
                write!(
                    f,
                    "Invalid {} count {} exceeds maximum possible elements ({}) for remaining data",
                    item, count, max_possible
                )
            }
            DecodeError::UnknownTag(b) => write!(f, "Unknown tag: 0x{:02X}", b),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<DecodeError> for PyErr {
    fn from(err: DecodeError) -> PyErr {
        pyo3::exceptions::PyValueError::new_err(err.to_string())
    }
}

pub struct DecodedGraph {
    pub strings: Vec<String>,
    pub records: Vec<Record>,
    pub type_registry: TypeRegistry,
}

pub fn decode(data: &[u8]) -> Result<DecodedGraph, DecodeError> {
    let mut offset = 0;

    let mut magic = [0u8; 4];
    magic.copy_from_slice(
        read_bytes(data, &mut offset, 4).ok_or(DecodeError::TooShort("header"))?,
    );
    if &magic != MAGIC_WRITE && &magic != MAGIC_LEGACY {
        return Err(DecodeError::InvalidMagic);
    }

    let _version = read_u16(data, &mut offset).ok_or(DecodeError::TooShort("version"))?;
    let _schema = read_u32(data, &mut offset).ok_or(DecodeError::TooShort("schema version"))?;
    let _flags = read_u8(data, &mut offset).ok_or(DecodeError::TooShort("flags"))?;

    let string_count =
        read_u32(data, &mut offset).ok_or(DecodeError::Truncated("string count"))?;
    let rem_bytes = data.len().saturating_sub(offset);
    let max_strings = rem_bytes / 4;
    if (string_count as usize) > max_strings {
        return Err(DecodeError::CountExceedsData {
            item: "string",
            count: string_count,
            max_possible: max_strings,
        });
    }

    let mut strings = Vec::with_capacity((string_count as usize).min(4096));
    for _ in 0..string_count {
        let len = read_u32(data, &mut offset)
            .ok_or(DecodeError::Truncated("string length"))? as usize;
        if len > data.len().saturating_sub(offset) {
            return Err(DecodeError::Truncated("string data"));
        }
        let bytes =
            read_bytes(data, &mut offset, len).ok_or(DecodeError::Truncated("string data"))?;
        let s = String::from_utf8(bytes.to_vec())
            .map_err(|e| DecodeError::InvalidUtf8(e.to_string()))?;
        strings.push(s);
    }

    let type_count = read_u32(data, &mut offset).ok_or(DecodeError::Truncated("type count"))?;
    let rem_bytes = data.len().saturating_sub(offset);
    let max_types = rem_bytes / 12;
    if (type_count as usize) > max_types {
        return Err(DecodeError::CountExceedsData {
            item: "type",
            count: type_count,
            max_possible: max_types,
        });
    }

    let mut type_registry = TypeRegistry::new();
    for _ in 0..type_count {
        let _type_id = read_u16(data, &mut offset).ok_or(DecodeError::Truncated("type_id"))?;
        let name_idx = read_u32(data, &mut offset)
            .ok_or(DecodeError::Truncated("type name_idx"))? as usize;
        let schema_version =
            read_u32(data, &mut offset).ok_or(DecodeError::Truncated("schema_version"))?;
        let field_count =
            read_u16(data, &mut offset).ok_or(DecodeError::Truncated("field_count"))?;
        let rem_f = data.len().saturating_sub(offset);
        let max_fields = rem_f / 4;
        if (field_count as usize) > max_fields {
            return Err(DecodeError::CountExceedsData {
                item: "field",
                count: field_count as u32,
                max_possible: max_fields,
            });
        }
        let mut fields = Vec::with_capacity((field_count as usize).min(4096));
        for _ in 0..field_count {
            let fidx = read_u32(data, &mut offset)
                .ok_or(DecodeError::Truncated("field index"))? as usize;
            fields.push(strings.get(fidx).cloned().unwrap_or_default());
        }
        let name = strings.get(name_idx).cloned().unwrap_or_default();
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        type_registry.register(&name, &field_refs, schema_version);
    }

    let obj_count = read_u32(data, &mut offset).ok_or(DecodeError::Truncated("object count"))?;
    let rem_bytes = data.len().saturating_sub(offset);
    let max_objects = rem_bytes;
    if (obj_count as usize) > max_objects {
        return Err(DecodeError::CountExceedsData {
            item: "object",
            count: obj_count,
            max_possible: max_objects,
        });
    }

    let mut records = Vec::with_capacity((obj_count as usize).min(4096));
    for _ in 0..obj_count {
        let tag_byte = read_u8(data, &mut offset).ok_or(DecodeError::Truncated("tag"))?;
        let tag = Tag::from_u8(tag_byte).ok_or(DecodeError::UnknownTag(tag_byte))?;

        let record = match tag {
            Tag::None => Record::None,
            Tag::True => Record::Bool(true),
            Tag::False => Record::Bool(false),
            Tag::Int => {
                let v = read_i64_zigzag(data, &mut offset)
                    .ok_or(DecodeError::Truncated("int"))?;
                Record::Int(v)
            }
            Tag::Float => {
                let bytes = read_bytes(data, &mut offset, 8)
                    .ok_or(DecodeError::Truncated("float"))?;
                Record::Float(f64::from_le_bytes(bytes.try_into().unwrap()))
            }
            Tag::String => {
                let idx = read_u32(data, &mut offset)
                    .ok_or(DecodeError::Truncated("string index"))?;
                Record::String(idx)
            }
            Tag::Bytes => {
                let len = read_u32(data, &mut offset)
                    .ok_or(DecodeError::Truncated("bytes length"))? as usize;
                if len > data.len().saturating_sub(offset) {
                    return Err(DecodeError::Truncated("bytes data"));
                }
                let bytes = read_bytes(data, &mut offset, len)
                    .ok_or(DecodeError::Truncated("bytes data"))?;
                Record::Bytes(bytes.to_vec())
            }
            Tag::List | Tag::Tuple => {
                let count = read_u32(data, &mut offset)
                    .ok_or(DecodeError::Truncated("list/tuple count"))? as usize;
                let rem_items = data.len().saturating_sub(offset);
                let max_refs = rem_items / 4;
                if count > max_refs {
                    return Err(DecodeError::CountExceedsData {
                        item: "list/tuple item",
                        count: count as u32,
                        max_possible: max_refs,
                    });
                }
                let mut refs = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    let r = read_u32(data, &mut offset)
                        .ok_or(DecodeError::Truncated("list/tuple ref"))?;
                    refs.push(r);
                }
                if tag == Tag::List {
                    Record::List(refs)
                } else {
                    Record::Tuple(refs)
                }
            }
            Tag::Dict => {
                let count = read_u32(data, &mut offset)
                    .ok_or(DecodeError::Truncated("dict count"))? as usize;
                let rem_pairs = data.len().saturating_sub(offset);
                let max_pairs = rem_pairs / 8;
                if count > max_pairs {
                    return Err(DecodeError::CountExceedsData {
                        item: "dict pair",
                        count: count as u32,
                        max_possible: max_pairs,
                    });
                }
                let mut pairs = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    let kr = read_u32(data, &mut offset)
                        .ok_or(DecodeError::Truncated("dict key"))?;
                    let vr = read_u32(data, &mut offset)
                        .ok_or(DecodeError::Truncated("dict value"))?;
                    pairs.push((kr, vr));
                }
                Record::Dict(pairs)
            }
            Tag::Set | Tag::FrozenSet => {
                let count = read_u32(data, &mut offset)
                    .ok_or(DecodeError::Truncated("set count"))? as usize;
                let rem_items = data.len().saturating_sub(offset);
                let max_refs = rem_items / 4;
                if count > max_refs {
                    return Err(DecodeError::CountExceedsData {
                        item: "set item",
                        count: count as u32,
                        max_possible: max_refs,
                    });
                }
                let mut refs = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    let r = read_u32(data, &mut offset)
                        .ok_or(DecodeError::Truncated("set ref"))?;
                    refs.push(r);
                }
                if tag == Tag::Set {
                    Record::Set(refs)
                } else {
                    Record::FrozenSet(refs)
                }
            }
            Tag::Dataclass => {
                let type_id = read_u16(data, &mut offset)
                    .ok_or(DecodeError::Truncated("dataclass type_id"))?;
                let field_count = read_u16(data, &mut offset)
                    .ok_or(DecodeError::Truncated("dataclass field_count"))?
                    as usize;
                let rem_f = data.len().saturating_sub(offset);
                let max_fields = rem_f / 4;
                if field_count > max_fields {
                    return Err(DecodeError::CountExceedsData {
                        item: "dataclass field",
                        count: field_count as u32,
                        max_possible: max_fields,
                    });
                }
                let mut fields = Vec::with_capacity(field_count.min(4096));
                for _ in 0..field_count {
                    let f = read_u32(data, &mut offset)
                        .ok_or(DecodeError::Truncated("dataclass field"))?;
                    fields.push(f);
                }
                Record::Dataclass { type_id, fields }
            }
            Tag::Reference => {
                let ref_id = read_u32(data, &mut offset)
                    .ok_or(DecodeError::Truncated("reference"))?;
                Record::Reference(ref_id)
            }
        };
        records.push(record);
    }

    Ok(DecodedGraph {
        strings,
        records,
        type_registry,
    })
}

enum WorkItem<'py> {
    Eval {
        ref_id: u32,
        depth: usize,
    },
    FinishList {
        ref_id: u32,
        list: Bound<'py, PyList>,
        count: usize,
    },
    FinishTuple {
        ref_id: u32,
        count: usize,
    },
    FinishDict {
        ref_id: u32,
        dict: Bound<'py, PyDict>,
        count: usize,
    },
    FinishSet {
        ref_id: u32,
        set: Bound<'py, PySet>,
        count: usize,
    },
    FinishFrozenSet {
        ref_id: u32,
        count: usize,
    },
    FinishDataclass {
        ref_id: u32,
        type_id: u16,
        count: usize,
    },
}

pub fn reconstruct<'py>(
    py: Python<'py>,
    decoded: &DecodedGraph,
    root_id: u32,
) -> PyResult<Bound<'py, PyAny>> {
    let mut memo: HashMap<u32, Bound<'py, PyAny>> = HashMap::new();
    let mut in_progress_immutables: HashSet<u32> = HashSet::new();
    let max_depth: usize = py
        .import("sys")
        .and_then(|sys| sys.getattr("getrecursionlimit"))
        .and_then(|func| func.call0())
        .and_then(|res| res.extract())
        .unwrap_or(1000);

    let mut eval_stack: Vec<Bound<'py, PyAny>> = Vec::new();
    let mut work_stack: Vec<WorkItem<'py>> = vec![WorkItem::Eval {
        ref_id: root_id,
        depth: 0,
    }];

    while let Some(item) = work_stack.pop() {
        match item {
            WorkItem::Eval { ref_id, depth } => {
                if depth >= max_depth {
                    return Err(pyo3::exceptions::PyRecursionError::new_err(
                        "maximum recursion depth exceeded during reconstruction",
                    ));
                }

                if in_progress_immutables.contains(&ref_id) {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "Cyclic reference involving immutable container at ref_id {} is not supported",
                        ref_id
                    )));
                }

                if let Some(obj) = memo.get(&ref_id) {
                    eval_stack.push(obj.clone());
                    continue;
                }

                let record = decoded.records.get(ref_id as usize).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(format!(
                        "Invalid reference: {}",
                        ref_id
                    ))
                })?;

                match record {
                    Record::None => {
                        let obj = py.None().into_bound(py);
                        memo.insert(ref_id, obj.clone());
                        eval_stack.push(obj);
                    }
                    Record::Bool(b) => {
                        let val: bool = *b;
                        let obj = val.into_pyobject(py)?.into_bound().into_any();
                        memo.insert(ref_id, obj.clone());
                        eval_stack.push(obj);
                    }
                    Record::Int(v) => {
                        let obj = v.into_pyobject(py)?.into_any();
                        memo.insert(ref_id, obj.clone());
                        eval_stack.push(obj);
                    }
                    Record::Float(v) => {
                        let obj = v.into_pyobject(py)?.into_any();
                        memo.insert(ref_id, obj.clone());
                        eval_stack.push(obj);
                    }
                    Record::String(idx) => {
                        let s = decoded.strings.get(*idx as usize).ok_or_else(|| {
                            pyo3::exceptions::PyValueError::new_err(format!(
                                "Invalid string index: {}",
                                idx
                            ))
                        })?;
                        let obj = s.as_str().into_pyobject(py)?.into_any();
                        memo.insert(ref_id, obj.clone());
                        eval_stack.push(obj);
                    }
                    Record::Bytes(data) => {
                        let py_bytes = PyBytes::new(py, data);
                        let obj = py_bytes.into_any();
                        memo.insert(ref_id, obj.clone());
                        eval_stack.push(obj);
                    }
                    Record::Reference(target_id) => {
                        work_stack.push(WorkItem::Eval {
                            ref_id: *target_id,
                            depth: depth + 1,
                        });
                    }
                    Record::List(refs) => {
                        let list = PyList::empty(py);
                        memo.insert(ref_id, list.clone().into_any());
                        work_stack.push(WorkItem::FinishList {
                            ref_id,
                            list,
                            count: refs.len(),
                        });
                        for r in refs.iter().rev() {
                            work_stack.push(WorkItem::Eval {
                                ref_id: *r,
                                depth: depth + 1,
                            });
                        }
                    }
                    Record::Tuple(refs) => {
                        in_progress_immutables.insert(ref_id);
                        work_stack.push(WorkItem::FinishTuple {
                            ref_id,
                            count: refs.len(),
                        });
                        for r in refs.iter().rev() {
                            work_stack.push(WorkItem::Eval {
                                ref_id: *r,
                                depth: depth + 1,
                            });
                        }
                    }
                    Record::Dict(pairs) => {
                        let dict = PyDict::new(py);
                        memo.insert(ref_id, dict.clone().into_any());
                        work_stack.push(WorkItem::FinishDict {
                            ref_id,
                            dict,
                            count: pairs.len(),
                        });
                        for (k, v) in pairs.iter().rev() {
                            work_stack.push(WorkItem::Eval {
                                ref_id: *v,
                                depth: depth + 1,
                            });
                            work_stack.push(WorkItem::Eval {
                                ref_id: *k,
                                depth: depth + 1,
                            });
                        }
                    }
                    Record::Set(refs) => {
                        let set = PySet::empty(py)?;
                        memo.insert(ref_id, set.clone().into_any());
                        work_stack.push(WorkItem::FinishSet {
                            ref_id,
                            set,
                            count: refs.len(),
                        });
                        for r in refs.iter().rev() {
                            work_stack.push(WorkItem::Eval {
                                ref_id: *r,
                                depth: depth + 1,
                            });
                        }
                    }
                    Record::FrozenSet(refs) => {
                        in_progress_immutables.insert(ref_id);
                        work_stack.push(WorkItem::FinishFrozenSet {
                            ref_id,
                            count: refs.len(),
                        });
                        for r in refs.iter().rev() {
                            work_stack.push(WorkItem::Eval {
                                ref_id: *r,
                                depth: depth + 1,
                            });
                        }
                    }
                    Record::Dataclass { type_id, fields } => {
                        work_stack.push(WorkItem::FinishDataclass {
                            ref_id,
                            type_id: *type_id,
                            count: fields.len(),
                        });
                        for f in fields.iter().rev() {
                            work_stack.push(WorkItem::Eval {
                                ref_id: *f,
                                depth: depth + 1,
                            });
                        }
                    }
                }
            }
            WorkItem::FinishList { list, count, .. } => {
                let start_idx = eval_stack.len().checked_sub(count).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Corrupted evaluation stack for list")
                })?;
                let items: Vec<Bound<'py, PyAny>> = eval_stack.drain(start_idx..).collect();
                for item in items {
                    list.append(item)?;
                }
                eval_stack.push(list.into_any());
            }
            WorkItem::FinishTuple { ref_id, count } => {
                in_progress_immutables.remove(&ref_id);
                let start_idx = eval_stack.len().checked_sub(count).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Corrupted evaluation stack for tuple")
                })?;
                let items: Vec<Bound<'py, PyAny>> = eval_stack.drain(start_idx..).collect();
                let tup = PyTuple::new(py, items)?;
                let bound_tup = tup.into_any();
                memo.insert(ref_id, bound_tup.clone());
                eval_stack.push(bound_tup);
            }
            WorkItem::FinishDict { dict, count, .. } => {
                let start_idx = eval_stack.len().checked_sub(2 * count).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Corrupted evaluation stack for dict")
                })?;
                let items: Vec<Bound<'py, PyAny>> = eval_stack.drain(start_idx..).collect();
                for chunk in items.chunks_exact(2) {
                    dict.set_item(&chunk[0], &chunk[1])?;
                }
                eval_stack.push(dict.into_any());
            }
            WorkItem::FinishSet { set, count, .. } => {
                let start_idx = eval_stack.len().checked_sub(count).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Corrupted evaluation stack for set")
                })?;
                let items: Vec<Bound<'py, PyAny>> = eval_stack.drain(start_idx..).collect();
                for item in items {
                    set.add(item)?;
                }
                eval_stack.push(set.into_any());
            }
            WorkItem::FinishFrozenSet { ref_id, count } => {
                in_progress_immutables.remove(&ref_id);
                let start_idx = eval_stack.len().checked_sub(count).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "Corrupted evaluation stack for frozenset",
                    )
                })?;
                let items: Vec<Bound<'py, PyAny>> = eval_stack.drain(start_idx..).collect();
                let fs = PyFrozenSet::new(py, items)?;
                let bound_fs = fs.into_any();
                memo.insert(ref_id, bound_fs.clone());
                eval_stack.push(bound_fs);
            }
            WorkItem::FinishDataclass {
                ref_id,
                type_id,
                count,
            } => {
                let start_idx = eval_stack.len().checked_sub(count).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "Corrupted evaluation stack for dataclass",
                    )
                })?;
                let field_values: Vec<Bound<'py, PyAny>> =
                    eval_stack.drain(start_idx..).collect();

                let ti = decoded.type_registry.get_type(type_id).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(format!(
                        "Invalid type_id: {}",
                        type_id
                    ))
                })?;

                let make_dataclass = py
                    .import("pysafe_pickle._reconstruct")
                    .or_else(|_| py.import("pygraph._reconstruct"))?;

                let field_names: Vec<Bound<'py, PyString>> =
                    ti.fields.iter().map(|f| PyString::new(py, f)).collect();
                let field_names_list = PyList::new(py, &field_names)?;

                let cls = make_dataclass
                    .getattr("make_class")?
                    .call1((&ti.name, &field_names_list))?;

                let kwargs = pyo3::types::PyDict::new(py);
                for (name, val) in ti.fields.iter().zip(field_values.iter()) {
                    kwargs.set_item(name, val)?;
                }

                let obj = cls.call((), Some(&kwargs))?;

                let serialized_version = ti.schema_version;
                let current_version: u32 = obj
                    .getattr("__pysafe_pickle_version__")
                    .or_else(|_| obj.getattr("__pygraph_version__"))
                    .and_then(|v| v.extract())
                    .unwrap_or(0);

                if serialized_version > 0
                    && current_version > 0
                    && serialized_version != current_version
                {
                    let migrations_mod = py
                        .import("pysafe_pickle.migrations")
                        .or_else(|_| py.import("pygraph.migrations"))?;
                    let state_dict = pyo3::types::PyDict::new(py);
                    for (name, val) in ti.fields.iter().zip(field_values.iter()) {
                        state_dict.set_item(name.as_str(), val)?;
                    }
                    state_dict.set_item("__pysafe_pickle_version__", serialized_version)?;
                    state_dict.set_item("__pygraph_version__", serialized_version)?;

                    let migrated = migrations_mod.getattr("apply_migrations")?.call1((
                        &ti.name,
                        &state_dict,
                        serialized_version,
                        current_version,
                    ))?;

                    let migrated_bound = migrated.into_bound();
                    let migrated_dict: &Bound<'py, PyDict> =
                        migrated_bound.downcast().map_err(|_| {
                            pyo3::exceptions::PyTypeError::new_err(
                                "Migration function must return a dict",
                            )
                        })?;
                    let new_kwargs = pyo3::types::PyDict::new(py);
                    for item in migrated_dict.iter() {
                        let key: String = item.0.extract()?;
                        if key != "__pysafe_pickle_version__"
                            && key != "__pygraph_version__"
                        {
                            new_kwargs.set_item(&key, item.1)?;
                        }
                    }

                    let new_obj = cls.call((), Some(&new_kwargs))?;

                    let extra = pyo3::types::PyDict::new(py);
                    let current_fields_bound = obj.getattr("__dataclass_fields__")?;
                    let current_fields: &Bound<'py, PyDict> =
                        current_fields_bound.downcast().map_err(|_| {
                            pyo3::exceptions::PyTypeError::new_err(
                                "__dataclass_fields__ is not a dict",
                            )
                        })?;
                    let current_field_names: Vec<String> = current_fields
                        .keys()
                        .iter()
                        .map(|k| k.extract())
                        .collect::<PyResult<_>>()?;

                    for item in migrated_dict.iter() {
                        let key: String = item.0.extract()?;
                        if key != "__pysafe_pickle_version__"
                            && key != "__pygraph_version__"
                            && !current_field_names.contains(&key)
                        {
                            extra.set_item(&key, item.1)?;
                        }
                    }
                    if extra.len() > 0 {
                        let _ = new_obj.setattr("__pysafe_pickle_extra__", &extra);
                        let _ = new_obj.setattr("__pygraph_extra__", extra);
                    }

                    memo.insert(ref_id, new_obj.clone());
                    eval_stack.push(new_obj);
                } else {
                    memo.insert(ref_id, obj.clone());
                    eval_stack.push(obj);
                }
            }
        }
    }

    eval_stack.pop().ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Reconstruction produced no value")
    })
}
