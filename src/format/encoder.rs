use crate::graph::traversal::{ObjectGraph, Record};
use crate::graph::types::TypeRegistry;
use super::*;

pub fn encode(graph: &mut ObjectGraph, type_registry: &TypeRegistry) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    write_header(&mut buf, 0);

    for ti in &type_registry.types {
        graph.intern_string(&ti.name);
        for fname in &ti.fields {
            graph.intern_string(fname);
        }
    }

    write_u32(&mut buf, graph.strings.len() as u32);
    for s in &graph.strings {
        write_u32(&mut buf, s.len() as u32);
        buf.extend_from_slice(s.as_bytes());
    }

    write_u32(&mut buf, type_registry.types.len() as u32);
    for (id, ti) in type_registry.types.iter().enumerate() {
        write_u16(&mut buf, id as u16);
        let name_idx = graph.string_index.get(&ti.name).copied().unwrap_or(0);
        write_u32(&mut buf, name_idx);
        write_u32(&mut buf, ti.schema_version);
        write_u16(&mut buf, ti.fields.len() as u16);
        for fname in &ti.fields {
            let fidx = graph.string_index.get(fname).copied().unwrap_or(0);
            write_u32(&mut buf, fidx);
        }
    }

    let valid_records: Vec<&Record> = graph.records.iter().filter_map(|r| r.as_ref()).collect();
    write_u32(&mut buf, valid_records.len() as u32);
    for record in valid_records {
        encode_record(&mut buf, record);
    }

    buf
}

fn encode_record(buf: &mut Vec<u8>, record: &Record) {
    match record {
        Record::None => buf.push(Tag::None as u8),
        Record::Bool(true) => buf.push(Tag::True as u8),
        Record::Bool(false) => buf.push(Tag::False as u8),
        Record::Int(v) => {
            buf.push(Tag::Int as u8);
            write_i64_zigzag(buf, *v);
        }
        Record::Float(v) => {
            buf.push(Tag::Float as u8);
            buf.extend_from_slice(&v.to_le_bytes());
        }
        Record::String(idx) => {
            buf.push(Tag::String as u8);
            write_u32(buf, *idx);
        }
        Record::Bytes(data) => {
            buf.push(Tag::Bytes as u8);
            write_u32(buf, data.len() as u32);
            buf.extend_from_slice(data);
        }
        Record::List(refs) => {
            buf.push(Tag::List as u8);
            write_u32(buf, refs.len() as u32);
            for r in refs {
                write_u32(buf, *r);
            }
        }
        Record::Tuple(refs) => {
            buf.push(Tag::Tuple as u8);
            write_u32(buf, refs.len() as u32);
            for r in refs {
                write_u32(buf, *r);
            }
        }
        Record::Dict(pairs) => {
            buf.push(Tag::Dict as u8);
            write_u32(buf, pairs.len() as u32);
            for (k, v) in pairs {
                write_u32(buf, *k);
                write_u32(buf, *v);
            }
        }
        Record::Set(refs) => {
            buf.push(Tag::Set as u8);
            write_u32(buf, refs.len() as u32);
            for r in refs {
                write_u32(buf, *r);
            }
        }
        Record::FrozenSet(refs) => {
            buf.push(Tag::FrozenSet as u8);
            write_u32(buf, refs.len() as u32);
            for r in refs {
                write_u32(buf, *r);
            }
        }
        Record::Dataclass { type_id, fields } => {
            buf.push(Tag::Dataclass as u8);
            write_u16(buf, *type_id);
            write_u16(buf, fields.len() as u16);
            for f in fields {
                write_u32(buf, *f);
            }
        }
        Record::Reference(ref_id) => {
            buf.push(Tag::Reference as u8);
            write_u32(buf, *ref_id);
        }
    }
}
