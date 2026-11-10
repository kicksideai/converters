// SPDX-License-Identifier: MIT
//! Image sample color spaces and the PDF functions their tint transforms use
//! (ISO 32000-1 8.6 and 7.10).

use serde::{Deserialize, Serialize};

/// The layout of a decoded image sample buffer: its color space and the
/// optional `/Decode` mapping from sample values to color components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleLayout {
    pub color_space: ColorSpace,
    pub decode: Option<Vec<f32>>,
}

/// A resolved image color space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColorSpace {
    /// DeviceGray or CalGray.
    Gray,
    /// DeviceRGB or CalRGB.
    Rgb,
    /// DeviceCMYK.
    Cmyk,
    /// ICC profile component ranges and its device approximation.
    Icc {
        alternate: Box<ColorSpace>,
        range: Vec<f32>,
    },
    /// The reserved colorants do not use a tint transform.
    Separation {
        name: String,
        tint: Option<(Box<ColorSpace>, Function)>,
    },
    Lab {
        white_point: [f32; 3],
        range: [f32; 4],
    },
    Indexed {
        base: Box<ColorSpace>,
        hival: u32,
        lookup: Vec<u8>,
    },
    /// DeviceN colorants rendered through the tint transform into the alternate space.
    DeviceN {
        components: usize,
        alternate: Box<ColorSpace>,
        tint_transform: Function,
    },
}

impl ColorSpace {
    pub fn components(&self) -> usize {
        match self {
            ColorSpace::Gray | ColorSpace::Indexed { .. } | ColorSpace::Separation { .. } => 1,
            ColorSpace::Icc { alternate, .. } => alternate.components(),
            ColorSpace::Rgb | ColorSpace::Lab { .. } => 3,
            ColorSpace::Cmyk => 4,
            ColorSpace::DeviceN { components, .. } => *components,
        }
    }

    /// The `/Decode` array a sample buffer uses when the image declares none.
    pub fn default_decode(&self, bits_per_component: u8) -> Vec<f32> {
        let mut decode = vec![0.0; 2 * self.components()];
        self.default_decode_into(bits_per_component, &mut decode);
        decode
    }

    fn default_decode_into(&self, bits: u8, decode: &mut [f32]) {
        for pair in decode.chunks_exact_mut(2) {
            pair.copy_from_slice(&[0.0, 1.0]);
        }
        match self {
            ColorSpace::Icc { range, .. } => decode.copy_from_slice(range),
            ColorSpace::Indexed { .. } => {
                decode.copy_from_slice(&[0.0, ((1u32 << bits) - 1) as f32])
            }
            ColorSpace::Lab { range, .. } => {
                decode.copy_from_slice(&[0.0, 100.0, range[0], range[1], range[2], range[3]])
            }
            _ => (),
        }
    }

    /// True when colors in this space are shades of gray.
    pub fn is_gray(&self) -> bool {
        match self {
            ColorSpace::Gray => true,
            ColorSpace::Separation { tint, .. } => tint.as_ref().is_none_or(|(s, _)| s.is_gray()),
            ColorSpace::Icc { alternate, .. } => alternate.is_gray(),
            ColorSpace::Indexed { base, .. }
            | ColorSpace::DeviceN {
                alternate: base, ..
            } => base.is_gray(),
            _ => false,
        }
    }

    /// Converts color components to sRGB in [0, 1].
    pub fn to_rgb(&self, components: &[f32]) -> Option<[f32; 3]> {
        let c = |i: usize| components.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        Some(match self {
            ColorSpace::Gray => [c(0); 3],
            ColorSpace::Icc { alternate, range } => {
                let mut values = [0.0; 32];
                let n = alternate.components();
                values[..n].copy_from_slice(&components[..n]);
                clip_to(&mut values[..n], range);
                alternate.to_rgb(&values[..n])?
            }
            ColorSpace::Separation { name, tint: None } => {
                if name == "All" {
                    [1.0 - c(0); 3]
                } else {
                    [1.0; 3]
                }
            }
            ColorSpace::Rgb => [c(0), c(1), c(2)],
            ColorSpace::Cmyk => {
                let k = c(3);
                [
                    (1.0 - c(0)) * (1.0 - k),
                    (1.0 - c(1)) * (1.0 - k),
                    (1.0 - c(2)) * (1.0 - k),
                ]
            }
            ColorSpace::Lab { white_point, .. } => lab_to_rgb(components, white_point),
            ColorSpace::Indexed {
                base,
                hival,
                lookup,
            } => {
                let n = base.components();
                let index = components
                    .first()
                    .copied()
                    .unwrap_or(0.0)
                    .round()
                    .clamp(0.0, *hival as f32) as usize;
                let entry = lookup.get(index * n..index * n + n).unwrap_or(&[]);
                let mut base_components = [0.0; 32];
                let mut decode = [0.0; 64];
                base.default_decode_into(8, &mut decode[..2 * n]);
                for (i, byte) in entry.iter().enumerate() {
                    let (lo, hi) = (decode[2 * i], decode[2 * i + 1]);
                    base_components[i] = lo + (*byte as f32 / 255.0) * (hi - lo);
                }
                base.to_rgb(&base_components[..n])?
            }
            ColorSpace::DeviceN {
                alternate,
                tint_transform,
                ..
            }
            | ColorSpace::Separation {
                tint: Some((alternate, tint_transform)),
                ..
            } => {
                let mut values = [0.0; 32];
                tint_transform.eval_into(components, &mut values[..alternate.components()])?;
                alternate.to_rgb(&values[..alternate.components()])?
            }
        })
    }
}

fn lab_to_rgb(components: &[f32], white_point: &[f32; 3]) -> [f32; 3] {
    let l = components.first().copied().unwrap_or(0.0);
    let a = components.get(1).copied().unwrap_or(0.0);
    let b = components.get(2).copied().unwrap_or(0.0);
    let g = |x: f32| {
        if x >= 6.0 / 29.0 {
            x * x * x
        } else {
            108.0 / 841.0 * (x - 4.0 / 29.0)
        }
    };
    let m = (l + 16.0) / 116.0;
    let x = white_point[0] * g(m + a / 500.0);
    let y = white_point[1] * g(m);
    let z = white_point[2] * g(m - b / 200.0);
    let linear = [
        3.2406 * x - 1.5372 * y - 0.4986 * z,
        -0.9689 * x + 1.8758 * y + 0.0415 * z,
        0.0557 * x - 0.2040 * y + 1.0570 * z,
    ];
    linear.map(|v| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    })
}

/// Reads the `bits`-wide big-endian unsigned sample starting at `bit_offset`.
pub fn read_sample(data: &[u8], bit_offset: usize, bits: u32) -> u32 {
    match bits {
        8 => data.get(bit_offset / 8).copied().unwrap_or(0) as u32,
        16 => {
            let i = bit_offset / 8;
            ((data.get(i).copied().unwrap_or(0) as u32) << 8)
                | data.get(i + 1).copied().unwrap_or(0) as u32
        }
        _ => {
            let mut value = 0u32;
            for bit in bit_offset..bit_offset + bits as usize {
                let byte = data.get(bit / 8).copied().unwrap_or(0);
                value = (value << 1) | ((byte >> (7 - bit % 8)) & 1) as u32;
            }
            value
        }
    }
}

fn interpolate(x: f32, x0: f32, x1: f32, y0: f32, y1: f32) -> f32 {
    if x1 == x0 {
        y0
    } else {
        y0 + (x - x0) * (y1 - y0) / (x1 - x0)
    }
}

fn clip_to(values: &mut [f32], range: &[f32]) {
    for (i, value) in values.iter_mut().enumerate() {
        if let (Some(lo), Some(hi)) = (range.get(2 * i), range.get(2 * i + 1)) {
            *value = value.clamp(*lo, *hi);
        }
    }
}

/// A PDF function (ISO 32000-1 7.10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Function {
    /// Type 0.
    Sampled {
        domain: Vec<f32>,
        range: Vec<f32>,
        size: Vec<u32>,
        bits_per_sample: u32,
        encode: Vec<f32>,
        decode: Vec<f32>,
        samples: Vec<u8>,
    },
    /// Type 2.
    Exponential {
        domain: [f32; 2],
        c0: Vec<f32>,
        c1: Vec<f32>,
        exponent: f32,
        range: Option<Vec<f32>>,
    },
    /// Type 3.
    Stitching {
        domain: [f32; 2],
        functions: Vec<Function>,
        bounds: Vec<f32>,
        encode: Vec<f32>,
        range: Option<Vec<f32>>,
    },
    /// Type 4.
    Calculator {
        domain: Vec<f32>,
        range: Vec<f32>,
        program: Vec<PsOp>,
    },
    /// An array of one-output functions, one per output component.
    Array(Vec<Function>),
}

impl Function {
    pub fn eval(&self, input: &[f32]) -> Option<Vec<f32>> {
        let mut output = vec![0.0; self.outputs()];
        self.eval_into(input, &mut output)?;
        Some(output)
    }

    pub fn outputs(&self) -> usize {
        match self {
            Self::Sampled { range, .. } | Self::Calculator { range, .. } => range.len() / 2,
            Self::Exponential { c0, .. } => c0.len(),
            Self::Stitching { functions, .. } => functions.first().map_or(0, Self::outputs),
            Self::Array(functions) => functions.len(),
        }
    }

    /// Evaluates into caller-owned output storage, without heap allocation.
    pub fn eval_into(&self, input: &[f32], out: &mut [f32]) -> Option<()> {
        if out.len() != self.outputs() {
            return None;
        }
        match self {
            Function::Sampled {
                domain,
                range,
                size,
                bits_per_sample,
                encode,
                decode,
                samples,
            } => eval_sampled(
                input,
                domain,
                range,
                size,
                *bits_per_sample,
                encode,
                decode,
                samples,
                out,
            ),
            Function::Exponential {
                domain,
                c0,
                c1,
                exponent,
                range,
            } => {
                let x = input
                    .first()
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(domain[0], domain[1]);
                let xn = x.powf(*exponent);
                for ((value, a), b) in out.iter_mut().zip(c0).zip(c1) {
                    *value = a + xn * (b - a);
                }
                if let Some(range) = range {
                    clip_to(out, range);
                }
            }
            Function::Stitching {
                domain,
                functions,
                bounds,
                encode,
                range,
            } => {
                let x = input
                    .first()
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(domain[0], domain[1]);
                let k = bounds
                    .iter()
                    .take_while(|b| x >= **b)
                    .count()
                    .min(functions.len().saturating_sub(1));
                let lo = if k == 0 { domain[0] } else { bounds[k - 1] };
                let hi = if k < bounds.len() {
                    bounds[k]
                } else {
                    domain[1]
                };
                let (e0, e1) = (
                    encode.get(2 * k).copied().unwrap_or(0.0),
                    encode.get(2 * k + 1).copied().unwrap_or(1.0),
                );
                functions[k].eval_into(&[interpolate(x, lo, hi, e0, e1)], out)?;
                if let Some(range) = range {
                    clip_to(out, range);
                }
            }

            Function::Calculator {
                domain,
                range,
                program,
            } => {
                let mut stack = PsStack::new();
                for (i, x) in input.iter().enumerate() {
                    let lo = domain.get(2 * i).copied().unwrap_or(f32::MIN);
                    let hi = domain.get(2 * i + 1).copied().unwrap_or(f32::MAX);
                    stack.push(PsValue::Number(x.clamp(lo, hi) as f64))?;
                }
                run_program(program, &mut stack)?;
                let start = stack.len().checked_sub(out.len())?;
                out.fill(0.0);
                for (value, result) in out.iter_mut().zip(&stack[start..]) {
                    *value = result.number() as f32;
                }
                clip_to(out, range);
            }
            Function::Array(functions) => {
                for (f, value) in functions.iter().zip(out) {
                    f.eval_into(input, std::slice::from_mut(value))?;
                }
            }
        }
        Some(())
    }
}

#[allow(clippy::too_many_arguments)]
fn eval_sampled(
    input: &[f32],
    domain: &[f32],
    range: &[f32],
    size: &[u32],
    bits: u32,
    encode: &[f32],
    decode: &[f32],
    samples: &[u8],
    out: &mut [f32],
) {
    let m = size.len();
    let n = range.len() / 2;
    let mut base = [0usize; 32];
    let mut frac = [0f32; 32];
    for i in 0..m {
        let x = input
            .get(i)
            .copied()
            .unwrap_or(0.0)
            .clamp(domain[2 * i], domain[2 * i + 1]);
        let max = (size[i].max(1) - 1) as f32;
        let (e0, e1) = (
            encode.get(2 * i).copied().unwrap_or(0.0),
            encode.get(2 * i + 1).copied().unwrap_or(max),
        );
        let e = interpolate(x, domain[2 * i], domain[2 * i + 1], e0, e1).clamp(0.0, max);
        base[i] = (e.floor() as usize).min(size[i].saturating_sub(2) as usize);
        frac[i] = e - base[i] as f32;
        if size[i] < 2 {
            base[i] = 0;
            frac[i] = 0.0;
        }
    }
    let max_sample = ((1u64 << bits) - 1) as f32;
    out.fill(0.0);
    for corner in 0..(1usize << m) {
        let mut weight = 1.0;
        let mut index = 0usize;
        let mut stride = 1usize;
        for i in 0..m {
            let upper = corner >> i & 1 == 1;
            weight *= if upper { frac[i] } else { 1.0 - frac[i] };
            index += (base[i] + upper as usize) * stride;
            stride *= size[i] as usize;
        }
        if weight == 0.0 {
            continue;
        }
        for (j, value) in out.iter_mut().enumerate() {
            let sample = read_sample(samples, (index * n + j) * bits as usize, bits) as f32;
            *value += weight * sample;
        }
    }
    for (j, value) in out.iter_mut().enumerate() {
        let (d0, d1) = (
            decode.get(2 * j).copied().unwrap_or(range[2 * j]),
            decode.get(2 * j + 1).copied().unwrap_or(range[2 * j + 1]),
        );
        *value = interpolate(*value, 0.0, max_sample, d0, d1);
    }
    clip_to(out, range);
}

/// A token of a type 4 (PostScript calculator) function program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PsOp {
    Number(f64),
    Bool(bool),
    Operator(PsOperator),
    If(Vec<PsOp>),
    IfElse(Vec<PsOp>, Vec<PsOp>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PsOperator {
    Abs,
    Add,
    Atan,
    Ceiling,
    Cos,
    Cvi,
    Cvr,
    Div,
    Exp,
    Floor,
    Idiv,
    Ln,
    Log,
    Mod,
    Mul,
    Neg,
    Round,
    Sin,
    Sqrt,
    Sub,
    Truncate,
    And,
    Bitshift,
    Eq,
    Ge,
    Gt,
    Le,
    Lt,
    Ne,
    Not,
    Or,
    Xor,
    Copy,
    Dup,
    Exch,
    Index,
    Pop,
    Roll,
}

impl PsOperator {
    fn from_name(name: &str) -> Option<Self> {
        use PsOperator::*;
        Some(match name {
            "abs" => Abs,
            "add" => Add,
            "atan" => Atan,
            "ceiling" => Ceiling,
            "cos" => Cos,
            "cvi" => Cvi,
            "cvr" => Cvr,
            "div" => Div,
            "exp" => Exp,
            "floor" => Floor,
            "idiv" => Idiv,
            "ln" => Ln,
            "log" => Log,
            "mod" => Mod,
            "mul" => Mul,
            "neg" => Neg,
            "round" => Round,
            "sin" => Sin,
            "sqrt" => Sqrt,
            "sub" => Sub,
            "truncate" => Truncate,
            "and" => And,
            "bitshift" => Bitshift,
            "eq" => Eq,
            "ge" => Ge,
            "gt" => Gt,
            "le" => Le,
            "lt" => Lt,
            "ne" => Ne,
            "not" => Not,
            "or" => Or,
            "xor" => Xor,
            "copy" => Copy,
            "dup" => Dup,
            "exch" => Exch,
            "index" => Index,
            "pop" => Pop,
            "roll" => Roll,
            _ => return None,
        })
    }
}

/// Parses a type 4 function program (`{ ... }`).
pub fn parse_calculator(source: &[u8]) -> Option<Vec<PsOp>> {
    let text = std::str::from_utf8(source).ok()?;
    let uncommented = text
        .split(['\r', '\n'])
        .map(|line| line.split('%').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let spaced = uncommented.replace('{', " { ").replace('}', " } ");
    let mut tokens = spaced.split_whitespace();
    if tokens.next()? != "{" {
        return None;
    }
    parse_block(&mut tokens)
}

fn parse_block<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<Vec<PsOp>> {
    let mut ops = Vec::new();
    let mut pending: Vec<Vec<PsOp>> = Vec::new();
    loop {
        let token = tokens.next()?;
        match token {
            "}" => return pending.is_empty().then_some(ops),
            "{" => pending.push(parse_block(tokens)?),
            "if" => {
                let body = pending.pop()?;
                if !pending.is_empty() {
                    return None;
                }
                ops.push(PsOp::If(body));
            }
            "ifelse" => {
                let otherwise = pending.pop()?;
                let then = pending.pop()?;
                if !pending.is_empty() {
                    return None;
                }
                ops.push(PsOp::IfElse(then, otherwise));
            }
            "true" => ops.push(PsOp::Bool(true)),
            "false" => ops.push(PsOp::Bool(false)),
            _ => {
                if !pending.is_empty() {
                    return None;
                }
                match token.parse::<f64>() {
                    Ok(number) => ops.push(PsOp::Number(number)),
                    Err(_) => ops.push(PsOp::Operator(PsOperator::from_name(token)?)),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PsValue {
    Number(f64),
    Bool(bool),
}

impl PsValue {
    fn number(self) -> f64 {
        match self {
            PsValue::Number(n) => n,
            PsValue::Bool(b) => b as i64 as f64,
        }
    }

    fn int(self) -> i64 {
        self.number() as i64
    }

    fn truth(self) -> bool {
        match self {
            PsValue::Bool(b) => b,
            PsValue::Number(n) => n != 0.0,
        }
    }
}

// PDF calculator functions have an operand stack limit of 100 (ISO 32000-1, Annex C).
struct PsStack {
    values: [PsValue; 100],
    len: usize,
}
impl PsStack {
    fn new() -> Self {
        Self {
            values: [PsValue::Number(0.0); 100],
            len: 0,
        }
    }
    fn push(&mut self, value: PsValue) -> Option<()> {
        if matches!(value, PsValue::Number(n) if !n.is_finite()) {
            return None;
        }
        *self.values.get_mut(self.len)? = value;
        self.len += 1;
        Some(())
    }
    fn pop(&mut self) -> Option<PsValue> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            Some(self.values[self.len])
        }
    }
}
impl std::ops::Deref for PsStack {
    type Target = [PsValue];
    fn deref(&self) -> &[PsValue] {
        &self.values[..self.len]
    }
}
impl std::ops::DerefMut for PsStack {
    fn deref_mut(&mut self) -> &mut [PsValue] {
        &mut self.values[..self.len]
    }
}

fn run_program(program: &[PsOp], stack: &mut PsStack) -> Option<()> {
    for op in program {
        match op {
            PsOp::Number(n) => stack.push(PsValue::Number(*n))?,
            PsOp::Bool(b) => stack.push(PsValue::Bool(*b))?,
            PsOp::If(body) => {
                if stack.pop()?.truth() {
                    run_program(body, stack)?;
                }
            }
            PsOp::IfElse(then, otherwise) => {
                if stack.pop()?.truth() {
                    run_program(then, stack)?;
                } else {
                    run_program(otherwise, stack)?;
                }
            }
            PsOp::Operator(operator) => apply(*operator, stack)?,
        }
    }
    Some(())
}

fn apply(operator: PsOperator, stack: &mut PsStack) -> Option<()> {
    use PsOperator::*;
    use PsValue::{Bool, Number};
    let pop = |stack: &mut PsStack| stack.pop();
    match operator {
        Abs | Ceiling | Cos | Cvi | Cvr | Floor | Ln | Log | Neg | Round | Sin | Sqrt
        | Truncate | Not => {
            let a = pop(stack)?;
            let x = a.number();
            stack.push(match operator {
                Abs => Number(x.abs()),
                Ceiling => Number(x.ceil()),
                Cos => Number(x.to_radians().cos()),
                Cvi | Truncate => Number(x.trunc()),
                Floor => Number(x.floor()),
                Ln => Number(x.ln()),
                Log => Number(x.log10()),
                Neg => Number(-x),
                Round => Number((x + 0.5).floor()),
                Sin => Number(x.to_radians().sin()),
                Sqrt => Number(x.sqrt()),
                Not => match a {
                    Bool(b) => Bool(!b),
                    Number(_) => Number(!a.int() as f64),
                },
                _ => Number(x),
            })?;
        }
        Add | Atan | Div | Exp | Idiv | Mod | Mul | Sub | And | Or | Xor | Bitshift | Eq | Ne
        | Ge | Gt | Le | Lt => {
            let b = pop(stack)?;
            let a = pop(stack)?;
            let (x, y) = (a.number(), b.number());
            stack.push(match operator {
                Add => Number(x + y),
                Atan => Number(x.atan2(y).to_degrees().rem_euclid(360.0)),
                Div => Number(x / y),
                Idiv => Number(a.int().checked_div(b.int())? as f64),
                Mod => Number(a.int().checked_rem(b.int())? as f64),
                Mul => Number(x * y),
                Sub => Number(x - y),
                And | Or | Xor => match (a, b) {
                    (Bool(p), Bool(q)) => Bool(match operator {
                        And => p && q,
                        Or => p || q,
                        _ => p ^ q,
                    }),
                    _ => Number(match operator {
                        And => a.int() & b.int(),
                        Or => a.int() | b.int(),
                        _ => a.int() ^ b.int(),
                    } as f64),
                },
                Bitshift => {
                    let shift = b.int();
                    let value = a.int() as u32;
                    let shifted = if !(-31..=31).contains(&shift) {
                        0
                    } else if shift >= 0 {
                        value << shift
                    } else {
                        value >> -shift
                    };
                    Number(shifted as i32 as f64)
                }
                Eq | Ne => {
                    let equal = match (a, b) {
                        (Bool(p), Bool(q)) => p == q,
                        (Number(p), Number(q)) => p == q,
                        _ => false,
                    };
                    Bool(if operator == Eq { equal } else { !equal })
                }
                Ge => Bool(x >= y),
                Gt => Bool(x > y),
                Le => Bool(x <= y),
                Lt => Bool(x < y),
                _ => Number(x.powf(y)),
            })?;
        }
        Copy => {
            let n = pop(stack)?.int().max(0) as usize;
            let start = stack.len().checked_sub(n)?;
            let end = stack.len();
            for i in start..end {
                let value = stack[i];
                stack.push(value)?;
            }
        }
        Dup => {
            let top = stack.last().copied()?;
            stack.push(top)?;
        }
        Exch => {
            let b = pop(stack)?;
            let a = pop(stack)?;
            stack.push(b)?;
            stack.push(a)?;
        }
        Index => {
            let n = pop(stack)?.int().max(0) as usize;
            let value = stack.len().checked_sub(n + 1).map(|i| stack[i])?;
            stack.push(value)?;
        }
        Pop => {
            stack.pop()?;
        }
        Roll => {
            let j = pop(stack)?.int();
            let n = pop(stack)?.int().max(0) as usize;
            if n > 0 && n <= stack.len() {
                let start = stack.len() - n;
                let shift = j.rem_euclid(n as i64) as usize;
                stack[start..].rotate_right(shift);
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod calculator_tests {
    use super::*;

    #[test]
    fn every_calculator_operator_matches_postscript() {
        for (program, expected) in [
            ("-2 abs", 2.),
            ("2 3 add", 5.),
            ("1 0 atan", 90.),
            ("1.2 ceiling", 2.),
            ("60 cos", 0.5),
            ("-1.8 cvi", -1.),
            ("2 cvr", 2.),
            ("5 2 div", 2.5),
            ("2 3 exp", 8.),
            ("-1.2 floor", -2.),
            ("-5 2 idiv", -2.),
            ("1 ln", 0.),
            ("100 log", 2.),
            ("5 2 mod", 1.),
            ("2 3 mul", 6.),
            ("2 neg", -2.),
            ("-1.5 round", -1.),
            ("30 sin", 0.5),
            ("4 sqrt", 2.),
            ("5 2 sub", 3.),
            ("-1.8 truncate", -1.),
            ("6 3 and", 2.),
            ("true false and", 0.),
            ("-1 -1 bitshift", 2147483647.),
            ("1 32 bitshift", 0.),
            ("-1 -32 bitshift", 0.),
            ("1073741824 1 bitshift", -2147483648.),
            ("2 2 eq", 1.),
            ("true 1 eq", 0.),
            ("false 0 ne", 1.),
            ("2 1 ge", 1.),
            ("2 1 gt", 1.),
            ("1 2 le", 1.),
            ("1 2 lt", 1.),
            ("1 2 ne", 1.),
            ("0 not", -1.),
            ("false not", 1.),
            ("4 2 or", 6.),
            ("false true or", 1.),
            ("7 3 xor", 4.),
            ("true true xor", 0.),
            ("1 2 2 copy add add add", 6.),
            ("3 dup mul", 9.),
            ("1 2 exch sub", 1.),
            ("3 4 1 index", 3.),
            ("1 2 pop", 1.),
            ("1 2 3 3 1 roll", 2.),
            ("true { 7 } if", 7.),
            ("false { 2 } { 3 } ifelse", 3.),
            ("true { 2 } { 3 } ifelse", 2.),
        ] {
            let program_ops = parse_calculator(format!("{{ {program} }}").as_bytes()).unwrap();
            let mut stack = PsStack::new();
            run_program(&program_ops, &mut stack).unwrap();
            let got = stack.pop().unwrap().number();
            assert!(
                (got - expected).abs() < 1e-5,
                "{program}: {got} != {expected}"
            );
        }
    }
}
