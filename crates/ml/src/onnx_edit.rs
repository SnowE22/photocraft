//! A DirectML-friendly rewrite of LaMa's ONNX graph, made on the protobuf bytes.
//!
//! The Carve export of LaMa (`lama_fp32.onnx`) computes its Fourier units' DFTs as `MatMul(A, X)`
//! with a 2-D cos/sin matrix `A[N, K]` (the output of a `Cos` or `Sin` node) and a 5-D tensor
//! `X[B, C, W, K, 1]` (the output of an `Unsqueeze`). DirectML's MatMul takes at most 4-D tensors,
//! so the whole session fails to build ("The parameter is incorrect"). Each such MatMul becomes
//! the same product in 4-D: `Unsqueeze(MatMul(Squeeze(X, -1), Transpose(A)), -1)`, which gives
//! the CPU provider's result up to float rounding. Everything else is copied byte for byte,
//! unknown fields included. Field numbers are ONNX's (`onnx/onnx.proto`).

use std::collections::HashMap;

use crate::MlError;

// ModelProto
const MODEL_GRAPH: u32 = 7;
// GraphProto
const GRAPH_NODE: u32 = 1;
const GRAPH_INITIALIZER: u32 = 5;
// NodeProto
const NODE_INPUT: u32 = 1;
const NODE_OUTPUT: u32 = 2;
const NODE_NAME: u32 = 3;
const NODE_OP_TYPE: u32 = 4;
const NODE_ATTRIBUTE: u32 = 5;
const NODE_DOMAIN: u32 = 7;
// AttributeProto
const ATTR_NAME: u32 = 1;
const ATTR_INTS: u32 = 8;
const ATTR_TYPE: u32 = 20;
const ATTR_TYPE_INTS: u64 = 7;
// TensorProto
const TENSOR_DIMS: u32 = 1;
const TENSOR_DATA_TYPE: u32 = 2;
const TENSOR_NAME: u32 = 8;
const TENSOR_RAW_DATA: u32 = 9;
const TENSOR_INT64: u64 = 7;

// Wire types.
const VARINT: u8 = 0;
const I64: u8 = 1;
const LEN: u8 = 2;
const I32: u8 = 5;

/// Prefix of every name the rewrite adds.
const PREFIX: &[u8] = b"photocraft_dml/";
/// The shared `[-1]` axes of the Squeezes and Unsqueezes.
const AXES: &[u8] = b"photocraft_dml/axes_minus_1";

/// One field of a message: its number and wire type, a varint's value or the payload of a
/// length-delimited or fixed-size field, and the whole field as it was encoded.
struct Field<'a> {
    num: u32,
    wire: u8,
    bytes: &'a [u8],
    raw: &'a [u8],
}

fn damaged(what: &str) -> MlError {
    MlError::Load(format!("the model file is damaged ({what})"))
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn varint(&mut self) -> Result<u64, MlError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *self.buf.get(self.pos).ok_or_else(|| damaged("a number runs past the end"))?;
            self.pos += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(damaged("a number is too long"))
    }

    fn take(&mut self, n: u64) -> Result<&'a [u8], MlError> {
        let n = usize::try_from(n).map_err(|_| damaged("a field is too long"))?;
        let end = self.pos.checked_add(n).ok_or_else(|| damaged("a field is too long"))?;
        let s = self.buf.get(self.pos..end).ok_or_else(|| damaged("a field runs past the end"))?;
        self.pos = end;
        Ok(s)
    }

    fn next(&mut self) -> Result<Option<Field<'a>>, MlError> {
        if self.pos >= self.buf.len() {
            return Ok(None);
        }
        let start = self.pos;
        let key = self.varint()?;
        let num = u32::try_from(key >> 3).map_err(|_| damaged("a field number is out of range"))?;
        let wire = (key & 7) as u8;
        let bytes = match wire {
            VARINT => {
                self.varint()?;
                &[][..]
            }
            I64 => self.take(8)?,
            LEN => {
                let n = self.varint()?;
                self.take(n)?
            }
            I32 => self.take(4)?,
            _ => return Err(damaged("a field of an unsupported type")),
        };
        let raw = self.buf.get(start..self.pos).ok_or_else(|| damaged("a field runs past the end"))?;
        Ok(Some(Field { num, wire, bytes, raw }))
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_key(out: &mut Vec<u8>, num: u32, wire: u8) {
    put_varint(out, (u64::from(num) << 3) | u64::from(wire));
}

fn put_len(out: &mut Vec<u8>, num: u32, bytes: &[u8]) {
    put_key(out, num, LEN);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn put_uint(out: &mut Vec<u8>, num: u32, v: u64) {
    put_key(out, num, VARINT);
    put_varint(out, v);
}

/// The parts of a NodeProto the rewrite looks at.
#[derive(Default)]
struct Node<'a> {
    inputs: Vec<&'a [u8]>,
    outputs: Vec<&'a [u8]>,
    name: &'a [u8],
    op: &'a [u8],
    domain: &'a [u8],
}

fn node(bytes: &[u8]) -> Result<Node<'_>, MlError> {
    let mut n = Node::default();
    let mut r = Reader::new(bytes);
    while let Some(f) = r.next()? {
        if f.wire != LEN {
            continue;
        }
        match f.num {
            NODE_INPUT => n.inputs.push(f.bytes),
            NODE_OUTPUT => n.outputs.push(f.bytes),
            NODE_NAME => n.name = f.bytes,
            NODE_OP_TYPE => n.op = f.bytes,
            NODE_DOMAIN => n.domain = f.bytes,
            _ => {}
        }
    }
    Ok(n)
}

fn put_node(out: &mut Vec<u8>, name: &[u8], op: &[u8], inputs: &[&[u8]], outputs: &[&[u8]], attributes: &[Vec<u8>]) {
    let mut n = Vec::new();
    for i in inputs {
        put_len(&mut n, NODE_INPUT, i);
    }
    for o in outputs {
        put_len(&mut n, NODE_OUTPUT, o);
    }
    put_len(&mut n, NODE_NAME, name);
    put_len(&mut n, NODE_OP_TYPE, op);
    for a in attributes {
        put_len(&mut n, NODE_ATTRIBUTE, a);
    }
    put_len(out, GRAPH_NODE, &n);
}

fn ints_attribute(name: &[u8], ints: &[i64]) -> Vec<u8> {
    let mut a = Vec::new();
    put_len(&mut a, ATTR_NAME, name);
    for i in ints {
        // int64 on the wire: two's complement in a varint.
        put_uint(&mut a, ATTR_INTS, *i as u64);
    }
    put_uint(&mut a, ATTR_TYPE, ATTR_TYPE_INTS);
    a
}

fn axes_initializer() -> Vec<u8> {
    let mut t = Vec::new();
    put_uint(&mut t, TENSOR_DIMS, 1);
    put_uint(&mut t, TENSOR_DATA_TYPE, TENSOR_INT64);
    put_len(&mut t, TENSOR_NAME, AXES);
    put_len(&mut t, TENSOR_RAW_DATA, &(-1i64).to_le_bytes());
    t
}

/// The model's one graph.
fn graph(model: &[u8]) -> Result<&[u8], MlError> {
    let mut r = Reader::new(model);
    let mut graph = None;
    while let Some(f) = r.next()? {
        if f.num == MODEL_GRAPH && f.wire == LEN {
            if graph.is_some() {
                return Err(damaged("more than one graph"));
            }
            graph = Some(f.bytes);
        }
    }
    graph.ok_or_else(|| damaged("no graph"))
}

/// The model with its 2-D × 5-D Fourier MatMuls made 4-D, and how many changed; `None` when none
/// needed it (a model already rewritten, or another model), so the bytes are used as they are.
pub fn for_directml(model: &[u8]) -> Result<Option<(Vec<u8>, usize)>, MlError> {
    let graph = graph(model)?;
    // What makes each value.
    let mut producer: HashMap<&[u8], &[u8]> = HashMap::new();
    let mut r = Reader::new(graph);
    while let Some(f) = r.next()? {
        if f.num == GRAPH_NODE && f.wire == LEN {
            let n = node(f.bytes)?;
            for o in &n.outputs {
                producer.insert(o, n.op);
            }
        }
    }
    let made_by = |name: Option<&&[u8]>, ops: &[&[u8]]| name.and_then(|n| producer.get(*n)).is_some_and(|op| ops.contains(op));

    let mut out_graph = Vec::with_capacity(graph.len().saturating_add(1 << 16));
    let mut changed = 0;
    let mut r = Reader::new(graph);
    while let Some(f) = r.next()? {
        if f.num == GRAPH_NODE && f.wire == LEN {
            let n = node(f.bytes)?;
            let fourier = n.op == b"MatMul"
                && (n.domain.is_empty() || n.domain == b"ai.onnx")
                && n.inputs.len() == 2
                && made_by(n.inputs.first(), &[b"Cos", b"Sin"])
                && made_by(n.inputs.get(1), &[b"Unsqueeze"]);
            if let (true, Some(a), Some(x), [y]) = (fourier, n.inputs.first(), n.inputs.get(1), n.outputs.as_slice()) {
                let base = if n.name.is_empty() { *y } else { n.name };
                let tag = |s: &[u8]| [PREFIX, base, b"/", s].concat();
                let (at, x4, y4) = (tag(b"AT"), tag(b"X4"), tag(b"Y4"));
                put_node(&mut out_graph, &tag(b"transpose"), b"Transpose", &[a], &[&at], &[ints_attribute(b"perm", &[1, 0])]);
                put_node(&mut out_graph, &tag(b"squeeze"), b"Squeeze", &[x, AXES], &[&x4], &[]);
                put_node(&mut out_graph, &tag(b"matmul"), b"MatMul", &[&x4, &at], &[&y4], &[]);
                put_node(&mut out_graph, &tag(b"unsqueeze"), b"Unsqueeze", &[&y4, AXES], &[y], &[]);
                changed += 1;
                continue;
            }
        }
        out_graph.extend_from_slice(f.raw);
    }
    if changed == 0 {
        return Ok(None);
    }
    put_len(&mut out_graph, GRAPH_INITIALIZER, &axes_initializer());

    let mut out = Vec::with_capacity(model.len().saturating_add(out_graph.len().saturating_sub(graph.len())).saturating_add(16));
    let mut r = Reader::new(model);
    while let Some(f) = r.next()? {
        if f.num == MODEL_GRAPH && f.wire == LEN {
            put_len(&mut out, MODEL_GRAPH, &out_graph);
        } else {
            out.extend_from_slice(f.raw);
        }
    }
    Ok(Some((out, changed)))
}

// GraphProto
const GRAPH_INPUT: u32 = 11;
const GRAPH_OUTPUT: u32 = 12;
// ValueInfoProto › TypeProto › TypeProto.Tensor › TensorShapeProto › Dimension
const VALUE_TYPE: u32 = 2;
const TYPE_TENSOR: u32 = 1;
const TENSOR_TYPE_SHAPE: u32 = 2;
const SHAPE_DIM: u32 = 1;
const DIM_PARAM: u32 = 2;

/// A field's new payload, or `None` to keep it.
type Replace<'f> = dyn FnMut(&[u8]) -> Result<Option<Vec<u8>>, MlError> + 'f;

/// `msg` with each length-delimited field `num` replaced by `f(payload)` (`None` keeps it); also
/// whether any was replaced. Other fields are copied byte for byte.
fn rewrite_field(msg: &[u8], num: u32, f: &mut Replace<'_>) -> Result<(Vec<u8>, bool), MlError> {
    let mut out = Vec::with_capacity(msg.len().saturating_add(64));
    let mut changed = false;
    let mut r = Reader::new(msg);
    while let Some(field) = r.next()? {
        if field.num == num && field.wire == LEN
            && let Some(new) = f(field.bytes)?
        {
            put_len(&mut out, num, &new);
            changed = true;
            continue;
        }
        out.extend_from_slice(field.raw);
    }
    Ok((out, changed))
}

/// A TensorShapeProto of 4 dimensions (N, C, H, W) with H and W named `h` and `w`.
fn symbolic_shape(shape: &[u8]) -> Result<Option<Vec<u8>>, MlError> {
    let mut dims = 0;
    let mut r = Reader::new(shape);
    while let Some(f) = r.next()? {
        dims += usize::from(f.num == SHAPE_DIM);
    }
    if dims != 4 {
        return Ok(None);
    }
    let mut i = 0;
    let (out, changed) = rewrite_field(shape, SHAPE_DIM, &mut |_| {
        i += 1;
        let name: &[u8] = match i {
            3 => b"h",
            4 => b"w",
            _ => return Ok(None),
        };
        let mut d = Vec::new();
        put_len(&mut d, DIM_PARAM, name);
        Ok(Some(d))
    })?;
    Ok(changed.then_some(out))
}

/// The model with its 4-D image inputs and outputs of any height and width (named `h` and `w`
/// instead of fixed sizes); `None` when there were none. Carve's LaMa export declares 512 × 512
/// but builds its Fourier matrices from the tensors' shapes, so it runs at any size: the Remove
/// tool's Quality mode runs it at 1024.
pub fn symbolic_size(model: &[u8]) -> Result<Option<Vec<u8>>, MlError> {
    let mut any = false;
    let (out, _) = rewrite_field(model, MODEL_GRAPH, &mut |graph| {
        let mut value_info = |vi: &[u8]| -> Result<Option<Vec<u8>>, MlError> {
            let (vi, changed) = rewrite_field(vi, VALUE_TYPE, &mut |ty| {
                let (ty, changed) = rewrite_field(ty, TYPE_TENSOR, &mut |t| {
                    let (t, changed) = rewrite_field(t, TENSOR_TYPE_SHAPE, &mut |s| symbolic_shape(s))?;
                    Ok(changed.then_some(t))
                })?;
                Ok(changed.then_some(ty))
            })?;
            Ok(changed.then_some(vi))
        };
        let (g, a) = rewrite_field(graph, GRAPH_INPUT, &mut value_info)?;
        let (g, b) = rewrite_field(&g, GRAPH_OUTPUT, &mut value_info)?;
        any |= a || b;
        Ok((a || b).then_some(g))
    })?;
    Ok(any.then_some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ValueInfoProto `name` of shape `dims` (a value, or a name).
    fn value_info(name: &str, dims: &[Result<u64, &str>]) -> Vec<u8> {
        let mut shape = Vec::new();
        for d in dims {
            let mut dim = Vec::new();
            match d {
                Ok(v) => put_uint(&mut dim, 1, *v),
                Err(p) => put_len(&mut dim, DIM_PARAM, p.as_bytes()),
            }
            put_len(&mut shape, SHAPE_DIM, &dim);
        }
        let mut tensor = Vec::new();
        put_uint(&mut tensor, 1, 1);
        put_len(&mut tensor, TENSOR_TYPE_SHAPE, &shape);
        let mut ty = Vec::new();
        put_len(&mut ty, TYPE_TENSOR, &tensor);
        let mut vi = Vec::new();
        put_len(&mut vi, 1, name.as_bytes());
        put_len(&mut vi, VALUE_TYPE, &ty);
        vi
    }

    /// The dims of every graph input and output, as text.
    fn io_dims(model: &[u8]) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        let mut r = Reader::new(graph(model).unwrap());
        while let Some(f) = r.next().unwrap() {
            if f.num != GRAPH_INPUT && f.num != GRAPH_OUTPUT {
                continue;
            }
            let mut dims = Vec::new();
            let field = |b: &[u8], n: u32| {
                let mut r = Reader::new(b);
                std::iter::from_fn(move || r.next().unwrap()).find(|f| f.num == n).map(|f| f.bytes.to_vec()).unwrap()
            };
            let shape = field(&field(&field(f.bytes, VALUE_TYPE), TYPE_TENSOR), TENSOR_TYPE_SHAPE);
            let mut r = Reader::new(&shape);
            while let Some(d) = r.next().unwrap() {
                let mut dr = Reader::new(d.bytes);
                let x = dr.next().unwrap().unwrap();
                dims.push(if x.num == DIM_PARAM {
                    String::from_utf8(x.bytes.to_vec()).unwrap()
                } else {
                    let mut vr = Reader::new(x.raw);
                    vr.varint().unwrap();
                    vr.varint().unwrap().to_string()
                });
            }
            out.push(dims);
        }
        out
    }

    #[test]
    fn image_inputs_and_outputs_get_any_height_and_width() {
        let mut g = Vec::new();
        put_len(&mut g, GRAPH_INPUT, &value_info("image", &[Err("batch"), Ok(3), Ok(512), Ok(512)]));
        put_len(&mut g, GRAPH_INPUT, &value_info("ids", &[Ok(1), Ok(77)]));
        put_len(&mut g, GRAPH_OUTPUT, &value_info("output", &[Err("batch"), Ok(3), Ok(512), Ok(512)]));
        let mut m = Vec::new();
        put_uint(&mut m, 1, 8);
        put_len(&mut m, MODEL_GRAPH, &g);
        let out = symbolic_size(&m).unwrap().expect("changed");
        assert_eq!(io_dims(&out), [vec!["batch", "3", "h", "w"], vec!["1", "77"], vec!["batch", "3", "h", "w"]]);
        // Already symbolic: rewritten the same way (idempotent), and a model without 4-D values
        // is left alone.
        assert_eq!(io_dims(&symbolic_size(&out).unwrap().unwrap()), io_dims(&out));
        let mut g = Vec::new();
        put_len(&mut g, GRAPH_INPUT, &value_info("ids", &[Ok(1), Ok(77)]));
        let mut m = Vec::new();
        put_len(&mut m, MODEL_GRAPH, &g);
        assert!(symbolic_size(&m).unwrap().is_none());
        // Damaged bytes are an error, not a panic.
        assert!(symbolic_size(&[0x3a, 0xff, 0xff]).is_err());
    }

    fn node_bytes(op: &str, inputs: &[&str], outputs: &[&str], name: &str) -> Vec<u8> {
        let mut n = Vec::new();
        for i in inputs {
            put_len(&mut n, NODE_INPUT, i.as_bytes());
        }
        for o in outputs {
            put_len(&mut n, NODE_OUTPUT, o.as_bytes());
        }
        put_len(&mut n, NODE_NAME, name.as_bytes());
        put_len(&mut n, NODE_OP_TYPE, op.as_bytes());
        n
    }

    /// A small model in LaMa's shape: a Fourier MatMul (Cos × Unsqueeze), one fed the other way
    /// round (Relu × Sin, already fine on DirectML), plus fields the rewrite must keep.
    fn model() -> Vec<u8> {
        let mut g = Vec::new();
        for n in [
            node_bytes("Cos", &["angles"], &["cos"], "cos"),
            node_bytes("Unsqueeze", &["x", "axes"], &["x5"], "unsq"),
            node_bytes("MatMul", &["cos", "x5"], &["y"], "dft"),
            node_bytes("Sin", &["angles"], &["sin"], "sin"),
            node_bytes("Relu", &["y"], &["r"], "relu"),
            node_bytes("MatMul", &["r", "sin"], &["z"], "idft"),
        ] {
            put_len(&mut g, GRAPH_NODE, &n);
        }
        put_len(&mut g, 2, b"graph-name");
        put_len(&mut g, 11, b"input-value-info");
        let mut m = Vec::new();
        put_uint(&mut m, 1, 8); // ir_version
        put_len(&mut m, 2, b"producer");
        put_len(&mut m, MODEL_GRAPH, &g);
        put_len(&mut m, 8, b"opset");
        m
    }

    /// A node's op, inputs and outputs.
    type Node = (String, Vec<String>, Vec<String>);

    /// Every node, and the initializers' names.
    fn nodes(model: &[u8]) -> (Vec<Node>, Vec<String>) {
        let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let mut r = Reader::new(graph(model).unwrap());
        let (mut nodes, mut inits) = (Vec::new(), Vec::new());
        while let Some(f) = r.next().unwrap() {
            if f.num == GRAPH_NODE {
                let n = node(f.bytes).unwrap();
                nodes.push((s(n.op), n.inputs.iter().map(|i| s(i)).collect(), n.outputs.iter().map(|o| s(o)).collect()));
            } else if f.num == GRAPH_INITIALIZER {
                let mut t = Reader::new(f.bytes);
                while let Some(tf) = t.next().unwrap() {
                    if tf.num == TENSOR_NAME {
                        inits.push(s(tf.bytes));
                    }
                }
            }
        }
        (nodes, inits)
    }

    #[test]
    fn a_fourier_matmul_becomes_four_dimensional() {
        let (out, n) = for_directml(&model()).unwrap().unwrap();
        assert_eq!(n, 1);
        let (nodes, inits) = nodes(&out);
        let ops: Vec<&str> = nodes.iter().map(|n| n.0.as_str()).collect();
        assert_eq!(ops, ["Cos", "Unsqueeze", "Transpose", "Squeeze", "MatMul", "Unsqueeze", "Sin", "Relu", "MatMul"]);
        let axes = String::from_utf8_lossy(AXES).into_owned();
        // Transpose(cos) -> AT; Squeeze(x5, axes) -> X4; MatMul(X4, AT) -> Y4; Unsqueeze(Y4, axes) -> y.
        let (t, sq, mm, un) = (&nodes[2], &nodes[3], &nodes[4], &nodes[5]);
        assert_eq!(t.1, ["cos"]);
        assert_eq!(sq.1, vec!["x5".to_string(), axes.clone()]);
        assert_eq!(mm.1, vec![sq.2[0].clone(), t.2[0].clone()]);
        assert_eq!(un.1, vec![mm.2[0].clone(), axes.clone()]);
        assert_eq!(un.2, ["y"]);
        assert!(t.2[0].starts_with("photocraft_dml/dft/"));
        // The other MatMul is untouched, and the axes are added once.
        assert_eq!(nodes[8].1, ["r", "sin"]);
        assert_eq!(inits, [axes]);
        // The model's and the graph's other fields are kept.
        for kept in [&b"producer"[..], b"opset", b"graph-name", b"input-value-info"] {
            assert!(out.windows(kept.len()).any(|w| w == kept));
        }
    }

    #[test]
    fn a_rewritten_or_unrelated_model_is_left_alone() {
        let (once, _) = for_directml(&model()).unwrap().unwrap();
        assert!(for_directml(&once).unwrap().is_none());
        let mut g = Vec::new();
        put_len(&mut g, GRAPH_NODE, &node_bytes("MatMul", &["a", "b"], &["c"], "plain"));
        let mut m = Vec::new();
        put_len(&mut m, MODEL_GRAPH, &g);
        assert!(for_directml(&m).unwrap().is_none());
    }

    #[test]
    fn the_axes_initializer_holds_minus_one() {
        let t = axes_initializer();
        let mut r = Reader::new(&t);
        let mut raw = None;
        while let Some(f) = r.next().unwrap() {
            if f.num == TENSOR_RAW_DATA {
                raw = Some(f.bytes.to_vec());
            }
        }
        assert_eq!(raw.unwrap(), (-1i64).to_le_bytes());
        // perm=[1, 0] round-trips through the varints.
        let a = ints_attribute(b"perm", &[1, 0, -1]);
        let mut r = Reader::new(&a);
        let mut ints = Vec::new();
        while let Some(f) = r.next().unwrap() {
            if f.num == ATTR_INTS {
                let mut v = Reader::new(&f.raw[1..]);
                ints.push(v.varint().unwrap() as i64);
            }
        }
        assert_eq!(ints, [1, 0, -1]);
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        assert!(for_directml(&[]).is_err());
        let m = model();
        for len in 0..m.len() {
            let _ = for_directml(&m[..len]);
        }
        for i in 0..m.len() {
            for b in [0x00, 0x07, 0x80, 0xff] {
                let mut x = m.clone();
                x[i] = b;
                let _ = for_directml(&x);
            }
        }
        // A varint longer than ten bytes, and a length beyond the buffer.
        assert!(for_directml(&[0x3a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]).is_err());
        assert!(for_directml(&[0x3a, 0x05, 0x00]).is_err());
        // Two graphs.
        let mut two = m.clone();
        two.extend_from_slice(&m);
        assert!(for_directml(&two).is_err());
    }
}
