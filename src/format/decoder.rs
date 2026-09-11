use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::*;
use pyo3::BoundObject;

use super::*;
use crate::graph::traversal::Record;
use crate::graph::types::TypeRegistry;

pub struct DecodedGraph {
    pub strings: Vec<String>,
    pub records: Vec<Record>,
    pub type_registry: TypeRegistry,
}

pub fn decode(data: &[u8]) -> PyResult<DecodedGraph> {
    let mut offset = 0;

    let mut magic = [0u8; 4];
    magic.copy_from_slice(read_bytes(data, &mut offset, 4).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: too short for header")
    })?);
    if &magic != MAGIC {
        return Err(pyo3::exceptions::PyValueError::new_err("Invalid magic bytes"));
    }

    let _version = read_u16(data, &mut offset).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: too short for version")
    })?;
    let _schema = read_u32(data, &mut offset).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: too short for schema version")
    })?;
    let _flags = read_u8(data, &mut offset).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: too short for flags")
    })?;

    let string_count = read_u32(data, &mut offset).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: no string count")
    })?;
    let mut strings = Vec::with_capacity(string_count as usize);
    for _ in 0..string_count {
        let len = read_u32(data, &mut offset).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Invalid data: string length truncated")
        })? as usize;
        let bytes = read_bytes(data, &mut offset, len).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Invalid data: string data truncated")
        })?;
        let s = String::from_utf8(bytes.to_vec()).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UTF-8 in string table: {}", e))
        })?;
        strings.push(s);
    }

    let type_count = read_u32(data, &mut offset).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: no type count")
    })?;
    let mut type_registry = TypeRegistry::new();
    for _ in 0..type_count {
        let _type_id = read_u16(data, &mut offset).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Invalid data: type_id truncated")
        })?;
        let name_idx = read_u32(data, &mut offset).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Invalid data: type name_idx truncated")
        })? as usize;
        let field_count = read_u16(data, &mut offset).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Invalid data: field_count truncated")
        })?;
        let mut fields = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            let fidx = read_u32(data, &mut offset).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("Invalid data: field index truncated")
            })? as usize;
            fields.push(strings.get(fidx).cloned().unwrap_or_default());
        }
        let name = strings.get(name_idx).cloned().unwrap_or_default();
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        type_registry.register(&name, &field_refs);
    }

    let obj_count = read_u32(data, &mut offset).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("Invalid data: no object count")
    })?;
    let mut records = Vec::with_capacity(obj_count as usize);
    for _ in 0..obj_count {
        let tag_byte = read_u8(data, &mut offset).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err("Invalid data: tag truncated")
        })?;
        let tag = Tag::from_u8(tag_byte).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!("Unknown tag: 0x{:02X}", tag_byte))
        })?;

        let record = match tag {
            Tag::None => Record::None,
            Tag::True => Record::Bool(true),
            Tag::False => Record::Bool(false),
            Tag::Int => {
                let v = read_i64_zigzag(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: int truncated")
                })?;
                Record::Int(v)
            }
            Tag::Float => {
                let bytes = read_bytes(data, &mut offset, 8).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: float truncated")
                })?;
                Record::Float(f64::from_le_bytes(bytes.try_into().unwrap()))
            }
            Tag::String => {
                let idx = read_u32(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: string index truncated")
                })?;
                Record::String(idx)
            }
            Tag::Bytes => {
                let len = read_u32(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: bytes length truncated")
                })? as usize;
                let bytes = read_bytes(data, &mut offset, len).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: bytes data truncated")
                })?;
                Record::Bytes(bytes.to_vec())
            }
            Tag::List | Tag::Tuple => {
                let count = read_u32(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: list count truncated")
                })? as usize;
                let mut refs = Vec::with_capacity(count);
                for _ in 0..count {
                    let r = read_u32(data, &mut offset).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err("Invalid data: list ref truncated")
                    })?;
                    refs.push(r);
                }
                if tag == Tag::List { Record::List(refs) } else { Record::Tuple(refs) }
            }
            Tag::Dict => {
                let count = read_u32(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: dict count truncated")
                })? as usize;
                let mut pairs = Vec::with_capacity(count);
                for _ in 0..count {
                    let kr = read_u32(data, &mut offset).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err("Invalid data: dict key truncated")
                    })?;
                    let vr = read_u32(data, &mut offset).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err("Invalid data: dict value truncated")
                    })?;
                    pairs.push((kr, vr));
                }
                Record::Dict(pairs)
            }
            Tag::Set | Tag::FrozenSet => {
                let count = read_u32(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: set count truncated")
                })? as usize;
                let mut refs = Vec::with_capacity(count);
                for _ in 0..count {
                    let r = read_u32(data, &mut offset).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err("Invalid data: set ref truncated")
                    })?;
                    refs.push(r);
                }
                if tag == Tag::Set { Record::Set(refs) } else { Record::FrozenSet(refs) }
            }
            Tag::Dataclass => {
                let type_id = read_u16(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: dataclass type_id truncated")
                })?;
                let field_count = read_u16(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: dataclass field_count truncated")
                })? as usize;
                let mut fields = Vec::with_capacity(field_count);
                for _ in 0..field_count {
                    let f = read_u32(data, &mut offset).ok_or_else(|| {
                        pyo3::exceptions::PyValueError::new_err("Invalid data: dataclass field truncated")
                    })?;
                    fields.push(f);
                }
                Record::Dataclass { type_id, fields }
            }
            Tag::Reference => {
                let ref_id = read_u32(data, &mut offset).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("Invalid data: reference truncated")
                })?;
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

pub fn reconstruct<'py>(
    py: Python<'py>,
    decoded: &DecodedGraph,
    root_id: u32,
) -> PyResult<Bound<'py, PyAny>> {
    let mut memo: HashMap<u32, Bound<'py, PyAny>> = HashMap::new();
    reconstruct_ref(py, decoded, root_id, &mut memo)
}

fn reconstruct_ref<'py>(
    py: Python<'py>,
    decoded: &DecodedGraph,
    ref_id: u32,
    memo: &mut HashMap<u32, Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(obj) = memo.get(&ref_id) {
        return Ok(obj.clone());
    }

    let record = decoded.records.get(ref_id as usize).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!("Invalid reference: {}", ref_id))
    })?;

    let obj = match record {
        Record::None => py.None().into_bound(py),
        Record::Bool(b) => {
            let val: bool = *b;
            val.into_pyobject(py)?.into_bound().into_any()
        }
        Record::Int(v) => v.into_pyobject(py)?.into_any(),
        Record::Float(v) => v.into_pyobject(py)?.into_any(),
        Record::String(idx) => {
            let s = decoded.strings.get(*idx as usize).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!("Invalid string index: {}", idx))
            })?;
            s.as_str().into_pyobject(py)?.into_any()
        }
        Record::Bytes(data) => {
            let py_bytes = PyBytes::new(py, data);
            py_bytes.into_any()
        }
        Record::List(refs) => {
            let list = PyList::empty(py);
            memo.insert(ref_id, list.clone().into_any());
            for r in refs {
                let item = reconstruct_ref(py, decoded, *r, memo)?;
                list.append(item)?;
            }
            list.into_any()
        }
        Record::Tuple(refs) => {
            let mut items = Vec::with_capacity(refs.len());
            let placeholder = PyTuple::empty(py);
            memo.insert(ref_id, placeholder.into_any());
            for r in refs {
                items.push(reconstruct_ref(py, decoded, *r, memo)?);
            }
            let tup = PyTuple::new(py, items)?;
            tup.into_any()
        }
        Record::Dict(pairs) => {
            let dict = PyDict::new(py);
            memo.insert(ref_id, dict.clone().into_any());
            for (k, v) in pairs {
                let key = reconstruct_ref(py, decoded, *k, memo)?;
                let val = reconstruct_ref(py, decoded, *v, memo)?;
                dict.set_item(key, val)?;
            }
            dict.into_any()
        }
        Record::Set(refs) => {
            let set = PySet::empty(py)?;
            memo.insert(ref_id, set.clone().into_any());
            for r in refs {
                let item = reconstruct_ref(py, decoded, *r, memo)?;
                set.add(item)?;
            }
            set.into_any()
        }
        Record::FrozenSet(refs) => {
            let mut items = Vec::with_capacity(refs.len());
            let placeholder = PyFrozenSet::empty(py)?;
            memo.insert(ref_id, placeholder.into_any());
            for r in refs {
                items.push(reconstruct_ref(py, decoded, *r, memo)?);
            }
            let fs = PyFrozenSet::new(py, items)?;
            fs.into_any()
        }
        Record::Dataclass { type_id, fields } => {
            let ti = decoded.type_registry.get_type(*type_id).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!("Invalid type_id: {}", type_id))
            })?;

            let _dataclasses = py.import("dataclasses")?;
            let make_dataclass = py.import("pygraph._reconstruct")?;

            let field_names: Vec<Bound<'py, PyString>> = ti
                .fields
                .iter()
                .map(|f| PyString::new(py, f))
                .collect();
            let field_names_list = PyList::new(py, &field_names)?;

            let field_values: Vec<Bound<'py, PyAny>> = fields
                .iter()
                .map(|f| reconstruct_ref(py, decoded, *f, memo))
                .collect::<PyResult<_>>()?;
            let _field_values_list = PyList::new(py, &field_values)?;

            let cls = make_dataclass.getattr("make_class")?.call1((
                &ti.name,
                &field_names_list,
            ))?;

            let kwargs = pyo3::types::PyDict::new(py);
            for (name, val) in ti.fields.iter().zip(field_values.iter()) {
                kwargs.set_item(name, val)?;
            }

            let obj = cls.call((), Some(&kwargs))?;
            memo.insert(ref_id, obj.clone());
            obj
        }
        Record::Reference(ref_id) => {
            return reconstruct_ref(py, decoded, *ref_id, memo);
        }
    };

    memo.insert(ref_id, obj.clone());
    Ok(obj)
}
