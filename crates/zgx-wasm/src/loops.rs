//! A budget of loop iterations for each shader invocation.
//!
//! A GPU preempts work between invocations (a pixel, a workgroup), not
//! inside one: one fragment shader looping for seconds holds the whole GPU
//! for seconds, zgx's own frames waiting behind it however many devices
//! there are. So each of a guest's WGSL modules is parsed here (naga, as
//! wgpu would) and given a private counter: every loop's body first adds
//! one to it and breaks out once it passes the budget. An invocation then
//! runs at most the budget's iterations in all, across its functions, and
//! the GPU can turn to zgx's work between them; the watchdog stops a guest
//! submitting many.
//!
//! A `break` the counter decides isn't uniform control flow, and WGSL wants
//! barriers and implicit derivatives (`textureSample`, `dpdx`) in uniform
//! control flow: a loop containing them (or calling a function that does)
//! is left alone, and if the module still doesn't validate so, every loop
//! of a function using them is. What can't be bounded so runs unbounded
//! (still under the watchdog), with a line in the session's log.
use naga::{
    AddressSpace, Arena, BinaryOperator, Block, Expression, Function, GlobalVariable, Handle,
    Literal, Module, Range, SampleLevel, Scalar, Span, Statement, Type, TypeInner,
    valid::{Capabilities, ValidationFlags, Validator},
};

/// Iterations a shader invocation may run, its loops' together (2¹⁶: a
/// ray marcher's hundreds and a blur's thousands, not a stall's billions).
pub const DEFAULT_BUDGET: u32 = 1 << 16;

/// `source` parsed, its loops bounded by `budget` (0: as written), and
/// whether every loop is (a line for the log if not).
pub(crate) fn bounded(source: &str, budget: u32) -> Result<(Module, Option<String>), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    if budget == 0 || !has_loops(&module) {
        return Ok((module, None));
    }
    // Invalid as written: wgpu says why, unchanged.
    if validate(&module).is_err() {
        return Ok((module, None));
    }
    for strict in [false, true] {
        let mut bounded = module.clone();
        let skipped = instrument(&mut bounded, budget, strict);
        if validate(&bounded).is_ok() {
            let note = (skipped > 0).then(|| {
                format!("{skipped} shader loops left unbounded (they need uniform control flow)")
            });
            return Ok((bounded, note));
        }
    }
    Ok((
        module,
        Some("a shader's loops couldn't be bounded: left as written".into()),
    ))
}

fn validate(module: &Module) -> Result<(), String> {
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .map(drop)
        .map_err(|e| e.to_string())
}

fn has_loops(module: &Module) -> bool {
    fn block(b: &Block) -> bool {
        b.iter().any(|s| match s {
            Statement::Loop { .. } => true,
            Statement::Block(b) => block(b),
            Statement::If { accept, reject, .. } => block(accept) || block(reject),
            Statement::Switch { cases, .. } => cases.iter().any(|c| block(&c.body)),
            _ => false,
        })
    }
    module.functions.iter().any(|(_, f)| block(&f.body))
        || module.entry_points.iter().any(|e| block(&e.function.body))
}

/// Count each loop's iterations against `budget`: the loops skipped
/// (needing uniform control flow; with `strict`, every loop of a function
/// needing it).
fn instrument(module: &mut Module, budget: u32, strict: bool) -> usize {
    let ty = module.types.insert(
        Type {
            name: None,
            inner: TypeInner::Scalar(Scalar::U32),
        },
        Span::UNDEFINED,
    );
    let zero = module
        .global_expressions
        .append(Expression::Literal(Literal::U32(0)), Span::UNDEFINED);
    let counter = module.global_variables.append(
        GlobalVariable {
            name: Some("zgx_loop_iterations".into()),
            space: AddressSpace::Private,
            binding: None,
            ty,
            init: Some(zero),
            memory_decorations: Default::default(),
        },
        Span::UNDEFINED,
    );
    // Which functions need uniform control flow, callees first (calls
    // reach only functions declared before them).
    let mut sensitive: Vec<bool> = Vec::new();
    for (_, function) in module.functions.iter() {
        let needs = block_needs_uniformity(&function.body, &function.expressions, &sensitive);
        sensitive.push(needs);
    }
    let mut skipped = 0;
    let mut bound = |function: &mut Function| {
        let whole = block_needs_uniformity(&function.body, &function.expressions, &sensitive);
        let mut body = std::mem::replace(&mut function.body, Block::new());
        let context = Context {
            counter,
            budget,
            sensitive: &sensitive,
            skip_all: strict && whole,
        };
        skipped += walk(&mut body, &mut function.expressions, &context);
        function.body = body;
    };
    for (_, function) in module.functions.iter_mut() {
        bound(function);
    }
    for entry in &mut module.entry_points {
        bound(&mut entry.function);
    }
    skipped
}

struct Context<'a> {
    counter: Handle<GlobalVariable>,
    budget: u32,
    sensitive: &'a [bool],
    skip_all: bool,
}

/// Bound the loops in `block`, innermost first: how many were skipped.
fn walk(block: &mut Block, expressions: &mut Arena<Expression>, cx: &Context<'_>) -> usize {
    let mut skipped = 0;
    for statement in block.iter_mut() {
        match statement {
            Statement::Block(b) => skipped += walk(b, expressions, cx),
            Statement::If { accept, reject, .. } => {
                skipped += walk(accept, expressions, cx);
                skipped += walk(reject, expressions, cx);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    skipped += walk(&mut case.body, expressions, cx);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                skipped += walk(body, expressions, cx);
                skipped += walk(continuing, expressions, cx);
                if cx.skip_all
                    || block_needs_uniformity(body, expressions, cx.sensitive)
                    || block_needs_uniformity(continuing, expressions, cx.sensitive)
                {
                    skipped += 1;
                } else {
                    count(body, expressions, cx);
                }
            }
            _ => {}
        }
    }
    skipped
}

/// `counter += 1; if counter > budget { break; }` ahead of `body`.
fn count(body: &mut Block, expressions: &mut Arena<Expression>, cx: &Context<'_>) {
    let span = Span::UNDEFINED;
    let pointer = expressions.append(Expression::GlobalVariable(cx.counter), span);
    let one = expressions.append(Expression::Literal(Literal::U32(1)), span);
    let budget = expressions.append(Expression::Literal(Literal::U32(cx.budget)), span);
    let load = expressions.append(Expression::Load { pointer }, span);
    let next = expressions.append(
        Expression::Binary {
            op: BinaryOperator::Add,
            left: load,
            right: one,
        },
        span,
    );
    let over = expressions.append(
        Expression::Binary {
            op: BinaryOperator::Greater,
            left: next,
            right: budget,
        },
        span,
    );
    let mut head = Block::new();
    head.push(Statement::Emit(Range::new_from_bounds(load, over)), span);
    head.push(
        Statement::Store {
            pointer,
            value: next,
        },
        span,
    );
    head.push(
        Statement::If {
            condition: over,
            accept: Block::from_vec(vec![Statement::Break]),
            reject: Block::new(),
        },
        span,
    );
    head.append(body);
    *body = head;
}

/// Whether `block` holds what wants uniform control flow: a barrier, a
/// workgroup-uniform load, a subgroup operation, an implicit derivative
/// (`textureSample`, `textureSampleBias`, `dpdx`), or a call of a function
/// that does.
fn block_needs_uniformity(
    block: &Block,
    expressions: &Arena<Expression>,
    sensitive: &[bool],
) -> bool {
    block.iter().any(|statement| match statement {
        Statement::ControlBarrier(_)
        | Statement::MemoryBarrier(_)
        | Statement::WorkGroupUniformLoad { .. }
        | Statement::SubgroupBallot { .. }
        | Statement::SubgroupGather { .. }
        | Statement::SubgroupCollectiveOperation { .. } => true,
        Statement::Call { function, .. } => {
            sensitive.get(function.index()).copied().unwrap_or(true)
        }
        Statement::Emit(range) => range.clone().any(|e| {
            matches!(
                expressions[e],
                Expression::ImageSample {
                    level: SampleLevel::Auto | SampleLevel::Bias(_),
                    ..
                } | Expression::Derivative { .. }
            )
        }),
        Statement::Block(b) => block_needs_uniformity(b, expressions, sensitive),
        Statement::If { accept, reject, .. } => {
            block_needs_uniformity(accept, expressions, sensitive)
                || block_needs_uniformity(reject, expressions, sensitive)
        }
        Statement::Switch { cases, .. } => cases
            .iter()
            .any(|c| block_needs_uniformity(&c.body, expressions, sensitive)),
        Statement::Loop {
            body, continuing, ..
        } => {
            block_needs_uniformity(body, expressions, sensitive)
                || block_needs_uniformity(continuing, expressions, sensitive)
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOPING: &str = r#"
@group(0) @binding(0) var<uniform> n: vec4<u32>;
fn spin(x0: f32) -> f32 {
    var x = x0;
    for (var i = 0u; i < n.x; i++) { x = fract(x * 1.618 + 0.318); }
    return x;
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    var x = spin(p.x);
    loop { x = x + 1.0; if x > 1e30 { break; } }
    return vec4<f32>(x, 0.0, 0.0, 1.0);
}
"#;

    #[test]
    fn every_loop_is_counted_and_the_module_validates() {
        let (module, note) = bounded(LOOPING, 1000).unwrap();
        assert_eq!(note, None);
        validate(&module).unwrap();
        let named = module
            .global_variables
            .iter()
            .any(|(_, g)| g.name.as_deref() == Some("zgx_loop_iterations"));
        assert!(named);
    }

    #[test]
    fn loops_needing_uniform_control_flow_are_left_alone() {
        let source = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    var c = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) { c += textureSample(t, s, p.xy * f32(i)); }
    var x = 0.0;
    for (var i = 0; i < 1000; i++) { x += 1.0; }
    return c + vec4<f32>(x);
}
"#;
        let (module, note) = bounded(source, 1000).unwrap();
        validate(&module).unwrap();
        assert!(
            note.is_some_and(|n| n.contains("unbounded")),
            "the sampling loop is noted"
        );
    }

    #[test]
    fn errors_are_wgsls() {
        let error = bounded("fn broken( {", 1000).unwrap_err();
        assert!(error.contains("error"), "{error}");
        assert!(
            bounded("@compute @workgroup_size(1) fn main() {}", 1000)
                .unwrap()
                .1
                .is_none()
        );
    }
}
