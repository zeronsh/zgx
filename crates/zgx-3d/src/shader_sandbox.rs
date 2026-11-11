//! Shaders that end: every loop of a shader counted against one budget an
//! invocation, for code the app doesn't vouch for (a plugin's compute
//! passes, materials and post passes). A shader that never ends holds the
//! device every window draws with until the operating system resets it,
//! and nothing else can stop one: a timestamp is only written when a pass
//! finishes.
//!
//! Why loops alone: WGSL has no recursion (naga's validator refuses a
//! function that calls itself, directly or not) and no `goto`, so the only
//! statement that runs more than once is a loop. naga lowers `for`, `while`
//! and `loop` alike to one `Statement::Loop` (`body`, `continuing`, and an
//! optional `break if`), and a function call is a statement like any other.
//! One counter for the invocation (a `var<private>`, which every invocation
//! has its own of, starting at zero), stepped at the top of every loop's
//! body in every function and breaking that loop past the cap, so bounds
//! all of an invocation's iterations together, however its loops nest or
//! call one another: once past the cap, every loop it's in, or enters,
//! breaks on its next iteration.
//!
//! A loop's `continuing` block can't `break`, but it runs only after its
//! body, which is where the check is; loops inside it are counted as any
//! other.
//!
//! What it refuses: a shader synchronising its invocations inside a loop
//! (`workgroupBarrier`, `storageBarrier`, `workgroupUniformLoad`, subgroup
//! operations), directly or in a function a loop calls. Invocations can
//! reach the cap at different iterations (their loops before it ran on
//! their own data), and one breaking out leaves the rest waiting at the
//! barrier: a hang, which is what this is for. naga's uniformity check
//! doesn't see it (the counter is a private variable), so it's refused
//! here. Barriers outside loops are kept: every invocation reaches them.
//! In a fragment shader, implicit derivatives (`textureSample`) in a loop
//! become undefined past the cap, not a hang.
use zgx_gpu::wgpu::naga;
// The same crate as wgpu's, with its WGSL writer (`Cargo.toml`).
use naga as naga_out;

/// The counter's name in the module.
const COUNTER: &str = "zgx_loop_budget";

/// What a sandboxed shader may cost and hold, an invocation and a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// IR nodes (expressions, statements, types, constants) in all: what
    /// validating and compiling it are proportional to.
    pub(crate) nodes: usize,
    /// Its entry point's cost (`cost`), calls expanded, loops once.
    pub(crate) cost: u64,
    /// A run's work: cost times invocations times the loop iterations each.
    pub(crate) work: u64,
    /// Bytes of private and function-local memory an invocation.
    pub(crate) memory: u64,
    /// Bytes of workgroup memory.
    pub(crate) workgroup: u64,
    /// The fewest loop iterations an invocation is left: fewer, refused.
    pub(crate) floor: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: 1 << 16,
            cost: 1 << 14,
            // About half a millisecond of light work on an M1 Max: 2^22
            // iterations of a twelve-node loop took 0.6 ms.
            work: 1 << 25,
            memory: 16 << 10,
            workgroup: 16 << 10,
            floor: 64,
        }
    }
}

/// A shader made safe to run: its module, loops bounded, and the
/// iterations each invocation may loop.
#[derive(Debug)]
pub(crate) struct Sandboxed {
    pub(crate) module: naga::Module,
    pub(crate) cap: u32,
    /// A run's work at most: cost times invocations times (cap + 1).
    pub(crate) work: u64,
}

/// `module`, whose `entry` runs `invocations` times a run, checked and
/// bounded: its size, memory and cost within `limits` (else why not), its
/// loops capped so a run's work stays within `limits.work`. For any stage:
/// a compute pass's invocations, or a fragment shader's pixels at most.
pub(crate) fn sandbox(
    module: naga::Module,
    entry: &str,
    invocations: u64,
    limits: &Limits,
) -> Result<Sandboxed, String> {
    let nodes = module.types.len()
        + module.constants.len()
        + module.global_expressions.len()
        + module
            .functions
            .iter()
            .map(|(_, f)| f)
            .chain(module.entry_points.iter().map(|e| &e.function))
            .map(|f| f.expressions.len() + statements(&f.body))
            .sum::<usize>();
    if nodes > limits.nodes {
        return Err(format!(
            "{nodes} IR nodes: a sandboxed shader has at most {}",
            limits.nodes
        ));
    }
    let (private, workgroup) = memory(&module)?;
    if private > limits.memory {
        return Err(format!(
            "{private} bytes of private and local memory an invocation: at most {}",
            limits.memory
        ));
    }
    if workgroup > limits.workgroup {
        return Err(format!(
            "{workgroup} bytes of workgroup memory: at most {}",
            limits.workgroup
        ));
    }
    let at = module
        .entry_points
        .iter()
        .position(|e| e.name == entry)
        .ok_or_else(|| format!("no entry point `{entry}`"))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| format!("{e}"))?;
    // One value moving a whole array: as large as an invocation's memory
    // at most, wherever it's from.
    let largest = largest_value(&module, &info);
    if largest > limits.memory {
        return Err(format!(
            "a value of {largest} bytes: a sandboxed shader's are at most {} (copy an array a part at a time)",
            limits.memory
        ));
    }
    let costs = cost(&module, &info)?;
    let cost = costs[at];
    if cost > limits.cost {
        return Err(format!(
            "its cost is {cost} (calls expanded): a sandboxed shader's is at most {}",
            limits.cost
        ));
    }
    let invocations = invocations.max(1);
    let looping = has_loops(&module);
    let per_iteration = cost.saturating_mul(invocations);
    let cap = if looping {
        // Iterations each, so cost x invocations x (cap + 1) fits the work.
        let cap = (limits.work / per_iteration.max(1)).saturating_sub(1);
        if cap < u64::from(limits.floor) {
            return Err(format!(
                "its loops would have {cap} iterations an invocation over {invocations} invocations \
                 at its cost of {cost}: at least {} are needed (fewer invocations, or a lighter shader)",
                limits.floor
            ));
        }
        cap.min(u64::from(u32::MAX)) as u32
    } else {
        if per_iteration > limits.work {
            return Err(format!(
                "{invocations} invocations at its cost of {cost} is {per_iteration}: a run's work is at most {}",
                limits.work
            ));
        }
        0
    };
    let work = per_iteration.saturating_mul(u64::from(cap) + 1);
    let module = bound_loops(module, cap.max(1))?;
    Ok(Sandboxed { module, cap, work })
}

/// Statements in `block`, its blocks' too.
fn statements(block: &naga::Block) -> usize {
    block
        .iter()
        .map(|statement| {
            1 + match statement {
                naga::Statement::Block(inner) => statements(inner),
                naga::Statement::If { accept, reject, .. } => {
                    statements(accept) + statements(reject)
                }
                naga::Statement::Switch { cases, .. } => {
                    cases.iter().map(|c| statements(&c.body)).sum()
                }
                naga::Statement::Loop {
                    body, continuing, ..
                } => statements(body) + statements(continuing),
                _ => 0,
            }
        })
        .sum()
}

/// Whether any function or entry point loops.
fn has_loops(module: &naga::Module) -> bool {
    fn block_loops(block: &naga::Block) -> bool {
        block.iter().any(|statement| match statement {
            naga::Statement::Loop { .. } => true,
            naga::Statement::Block(inner) => block_loops(inner),
            naga::Statement::If { accept, reject, .. } => {
                block_loops(accept) || block_loops(reject)
            }
            naga::Statement::Switch { cases, .. } => cases.iter().any(|c| block_loops(&c.body)),
            _ => false,
        })
    }
    module
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(module.entry_points.iter().map(|e| &e.function))
        .any(|f| block_loops(&f.body))
}

/// An invocation's private and function-local bytes (every function's
/// locals together, as if all were live at once), and the workgroup's.
fn memory(module: &naga::Module) -> Result<(u64, u64), String> {
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|e| format!("its types: {e}"))?;
    let size = |ty: naga::Handle<naga::Type>| u64::from(layouter[ty].size);
    let (mut private, mut workgroup) = (0u64, 0u64);
    for (_, global) in module.global_variables.iter() {
        match global.space {
            naga::AddressSpace::Private => private = private.saturating_add(size(global.ty)),
            naga::AddressSpace::WorkGroup => workgroup = workgroup.saturating_add(size(global.ty)),
            _ => {}
        }
    }
    for function in module
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(module.entry_points.iter().map(|e| &e.function))
    {
        for (_, local) in function.local_variables.iter() {
            private = private.saturating_add(size(local.ty));
        }
    }
    Ok((private, workgroup))
}

/// What a shader may cost, statically, before its loops repeat: every
/// value of an entry point and of every function it calls (each call site
/// counted with its callee's whole cost, so calls are expanded, as a driver
/// inlining them would), each loop's body once. A value weighs its size in
/// 16-byte units, at least 1 (a pointer 1): loading, storing, composing,
/// selecting or passing a whole array moves all of it. A texture sample
/// weighs 16 more, another image access 4, an atomic 8, a derivative 4.
/// Saturating. Its loops then repeat at most `cap` times all told an
/// invocation (`bound_loops`), so an invocation's work is at most this
/// times `cap + 1`.
///
/// Each function is weighed once, after those it calls, however the module
/// orders them: the call graph walked depth first with a memo, so the
/// weighing is linear in the module's size. An error where a function
/// calls itself, directly or not (naga's validator refuses that too).
pub(crate) fn cost(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
) -> Result<Vec<u64>, String> {
    let handles: Vec<naga::Handle<naga::Function>> =
        module.functions.iter().map(|(h, _)| h).collect();
    let count = handles.len();
    let mut costs: Vec<Option<u64>> = vec![None; count];
    // 0 not met, 1 on the path being walked, 2 weighed.
    let mut state = vec![0u8; count];
    for &start in &handles {
        if state[start.index()] == 2 {
            continue;
        }
        let mut stack = vec![(start, false)];
        while let Some((at, after)) = stack.pop() {
            let i = at.index();
            if after {
                let function = &module.functions[at];
                costs[i] = Some(function_cost(module, function, &info[at], &costs));
                state[i] = 2;
                continue;
            }
            match state[i] {
                2 => continue,
                1 => return Err("a function calls itself".into()),
                _ => {}
            }
            state[i] = 1;
            stack.push((at, true));
            let mut callees = Vec::new();
            calls(&module.functions[at].body, &mut callees);
            for callee in callees {
                match state[callee.index()] {
                    1 => return Err("a function calls itself".into()),
                    0 => stack.push((callee, false)),
                    _ => {}
                }
            }
        }
    }
    Ok(module
        .entry_points
        .iter()
        .enumerate()
        .map(|(at, entry)| function_cost(module, &entry.function, info.get_entry_point(at), &costs))
        .collect())
}

/// The functions `block` calls.
fn calls(block: &naga::Block, out: &mut Vec<naga::Handle<naga::Function>>) {
    for statement in block.iter() {
        match statement {
            naga::Statement::Call { function, .. } => out.push(*function),
            naga::Statement::Block(inner) => calls(inner, out),
            naga::Statement::If { accept, reject, .. } => {
                calls(accept, out);
                calls(reject, out);
            }
            naga::Statement::Switch { cases, .. } => {
                for case in cases {
                    calls(&case.body, out);
                }
            }
            naga::Statement::Loop {
                body, continuing, ..
            } => {
                calls(body, out);
                calls(continuing, out);
            }
            _ => {}
        }
    }
}

/// A value's bytes (a pointer's, none: what it points to isn't moved).
fn bytes(module: &naga::Module, inner: &naga::TypeInner) -> u64 {
    match inner {
        naga::TypeInner::Pointer { .. } | naga::TypeInner::ValuePointer { .. } => 0,
        _ => inner.try_size(module.to_ctx()).map_or(u64::MAX, u64::from),
    }
}

/// A value's weight: its bytes in 16-byte units, at least 1.
fn weight(module: &naga::Module, inner: &naga::TypeInner) -> u64 {
    bytes(module, inner).div_ceil(16).max(1)
}

/// The largest single value any function of `module` makes, in bytes.
pub(crate) fn largest_value(module: &naga::Module, info: &naga::valid::ModuleInfo) -> u64 {
    let of = |function: &naga::Function, fi: &naga::valid::FunctionInfo| {
        function
            .expressions
            .iter()
            .map(|(h, _)| bytes(module, fi[h].ty.inner_with(&module.types)))
            .max()
            .unwrap_or(0)
    };
    let functions = module.functions.iter().map(|(h, f)| of(f, &info[h]));
    let entries = module
        .entry_points
        .iter()
        .enumerate()
        .map(|(at, e)| of(&e.function, info.get_entry_point(at)));
    functions.chain(entries).max().unwrap_or(0)
}

/// `function`'s cost, those it calls weighed in `costs`.
fn function_cost(
    module: &naga::Module,
    function: &naga::Function,
    fi: &naga::valid::FunctionInfo,
    costs: &[Option<u64>],
) -> u64 {
    let value =
        |h: naga::Handle<naga::Expression>| weight(module, fi[h].ty.inner_with(&module.types));
    let expressions = function
        .expressions
        .iter()
        .map(|(h, e)| {
            let extra = match e {
                naga::Expression::ImageSample { .. } => 16,
                naga::Expression::ImageLoad { .. } | naga::Expression::ImageQuery { .. } => 4,
                naga::Expression::Derivative { .. } => 4,
                _ => 0,
            };
            value(h).saturating_add(extra)
        })
        .fold(0, u64::saturating_add);
    expressions.saturating_add(block_cost(&function.body, &value, costs))
}

fn block_cost(
    block: &naga::Block,
    value: &dyn Fn(naga::Handle<naga::Expression>) -> u64,
    costs: &[Option<u64>],
) -> u64 {
    let mut total = 0u64;
    for statement in block.iter() {
        let cost = match statement {
            // Its arguments passed by value, its callee's work, its result.
            naga::Statement::Call {
                function,
                arguments,
                ..
            } => arguments
                .iter()
                .map(|&a| value(a))
                .fold(1u64, u64::saturating_add)
                .saturating_add(costs[function.index()].unwrap_or(u64::MAX)),
            naga::Statement::Store { value: stored, .. } => value(*stored),
            naga::Statement::Return {
                value: Some(returned),
            } => value(*returned),
            naga::Statement::Atomic { .. } | naga::Statement::ImageAtomic { .. } => 8,
            naga::Statement::ImageStore { .. } => 4,
            naga::Statement::Block(inner) => block_cost(inner, value, costs),
            naga::Statement::If { accept, reject, .. } => 1u64
                .saturating_add(block_cost(accept, value, costs))
                .saturating_add(block_cost(reject, value, costs)),
            naga::Statement::Switch { cases, .. } => cases
                .iter()
                .map(|case| block_cost(&case.body, value, costs))
                .fold(1u64, u64::saturating_add),
            naga::Statement::Loop {
                body, continuing, ..
            } => 1u64
                .saturating_add(block_cost(body, value, costs))
                .saturating_add(block_cost(continuing, value, costs)),
            _ => 1,
        };
        total = total.saturating_add(cost);
    }
    total
}

/// `module` with every loop's iterations counted against `cap` an
/// invocation, all loops together: past it, each loop breaks. A shader that
/// never reaches the cap runs as it did (the counter is its own private
/// variable, and is read nowhere else). Refused (why) where a loop
/// synchronises invocations.
pub(crate) fn bound_loops(mut module: naga::Module, cap: u32) -> Result<naga::Module, String> {
    // Which functions synchronise, themselves or through those they call.
    let mut syncing = vec![false; module.functions.len()];
    loop {
        let mut changed = false;
        for (handle, function) in module.functions.iter() {
            let at = handle.index();
            if !syncing[at] && block_syncs(&function.body, &syncing) {
                syncing[at] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let functions = module.functions.iter().map(|(_, f)| f);
    let entries = module.entry_points.iter().map(|e| &e.function);
    for function in functions.chain(entries) {
        if loop_syncs(&function.body, &syncing) {
            return Err(format!(
                "`{}` synchronises invocations inside a loop: a sandboxed shader can't \
                 (its loops are bounded, and one invocation leaving a loop early would \
                 leave the rest waiting)",
                function.name.as_deref().unwrap_or("a function")
            ));
        }
    }
    let span = naga::Span::UNDEFINED;
    let u32_type = module.types.insert(
        naga::Type {
            name: None,
            inner: naga::TypeInner::Scalar(naga::Scalar::U32),
        },
        span,
    );
    let counter = module.global_variables.append(
        naga::GlobalVariable {
            name: Some(COUNTER.into()),
            space: naga::AddressSpace::Private,
            binding: None,
            ty: u32_type,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        },
        span,
    );
    for (_, function) in module.functions.iter_mut() {
        bound_function(function, counter, cap);
    }
    for entry in module.entry_points.iter_mut() {
        bound_function(&mut entry.function, counter, cap);
    }
    Ok(module)
}

/// Whether `block` synchronises invocations, itself or through a call to a
/// function `syncing` says does.
fn block_syncs(block: &naga::Block, syncing: &[bool]) -> bool {
    block.iter().any(|statement| match statement {
        naga::Statement::ControlBarrier(_)
        | naga::Statement::MemoryBarrier(_)
        | naga::Statement::WorkGroupUniformLoad { .. }
        | naga::Statement::SubgroupBallot { .. }
        | naga::Statement::SubgroupGather { .. }
        | naga::Statement::SubgroupCollectiveOperation { .. } => true,
        naga::Statement::Call { function, .. } => syncing[function.index()],
        naga::Statement::Block(inner) => block_syncs(inner, syncing),
        naga::Statement::If { accept, reject, .. } => {
            block_syncs(accept, syncing) || block_syncs(reject, syncing)
        }
        naga::Statement::Switch { cases, .. } => {
            cases.iter().any(|case| block_syncs(&case.body, syncing))
        }
        naga::Statement::Loop {
            body, continuing, ..
        } => block_syncs(body, syncing) || block_syncs(continuing, syncing),
        _ => false,
    })
}

/// Whether a loop in `block` synchronises invocations.
fn loop_syncs(block: &naga::Block, syncing: &[bool]) -> bool {
    block.iter().any(|statement| match statement {
        naga::Statement::Loop {
            body, continuing, ..
        } => block_syncs(body, syncing) || block_syncs(continuing, syncing),
        naga::Statement::Block(inner) => loop_syncs(inner, syncing),
        naga::Statement::If { accept, reject, .. } => {
            loop_syncs(accept, syncing) || loop_syncs(reject, syncing)
        }
        naga::Statement::Switch { cases, .. } => {
            cases.iter().any(|case| loop_syncs(&case.body, syncing))
        }
        _ => false,
    })
}

fn bound_function(
    function: &mut naga::Function,
    counter: naga::Handle<naga::GlobalVariable>,
    cap: u32,
) {
    let mut body = std::mem::take(&mut function.body);
    bound_block(&mut body, &mut function.expressions, counter, cap);
    function.body = body;
}

fn bound_block(
    block: &mut naga::Block,
    expressions: &mut naga::Arena<naga::Expression>,
    counter: naga::Handle<naga::GlobalVariable>,
    cap: u32,
) {
    for statement in block.iter_mut() {
        match statement {
            naga::Statement::Block(inner) => bound_block(inner, expressions, counter, cap),
            naga::Statement::If { accept, reject, .. } => {
                bound_block(accept, expressions, counter, cap);
                bound_block(reject, expressions, counter, cap);
            }
            naga::Statement::Switch { cases, .. } => {
                for case in cases.iter_mut() {
                    bound_block(&mut case.body, expressions, counter, cap);
                }
            }
            naga::Statement::Loop {
                body, continuing, ..
            } => {
                bound_block(body, expressions, counter, cap);
                bound_block(continuing, expressions, counter, cap);
                let mut checked = check(expressions, counter, cap);
                checked.append(body);
                *body = checked;
            }
            _ => {}
        }
    }
}

/// `module` validated and written as WGSL, for the device to take as any
/// shader: an error if it doesn't validate (a barrier or derivative in a
/// loop of a bounded shader, see the module's docs).
pub(crate) fn to_wgsl(module: &naga::Module) -> Result<String, String> {
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(module)
    .map_err(|e| format!("{e}"))?;
    naga_out::back::wgsl::write_string(module, &info, naga_out::back::wgsl::WriterFlags::empty())
        .map_err(|e| e.to_string())
}

/// `counter += 1; if counter > cap { break; }`, its expressions added to
/// `expressions`: what each loop's body begins with.
fn check(
    expressions: &mut naga::Arena<naga::Expression>,
    counter: naga::Handle<naga::GlobalVariable>,
    cap: u32,
) -> naga::Block {
    let span = naga::Span::UNDEFINED;
    // Those needing no emitting first; then the emitted run, together.
    let pointer = expressions.append(naga::Expression::GlobalVariable(counter), span);
    let one = expressions.append(naga::Expression::Literal(naga::Literal::U32(1)), span);
    let most = expressions.append(naga::Expression::Literal(naga::Literal::U32(cap)), span);
    let load = expressions.append(naga::Expression::Load { pointer }, span);
    let stepped = expressions.append(
        naga::Expression::Binary {
            op: naga::BinaryOperator::Add,
            left: load,
            right: one,
        },
        span,
    );
    let past = expressions.append(
        naga::Expression::Binary {
            op: naga::BinaryOperator::Greater,
            left: stepped,
            right: most,
        },
        span,
    );
    let mut block = naga::Block::new();
    block.push(
        naga::Statement::Emit(naga::Range::new_from_bounds(load, past)),
        span,
    );
    block.push(
        naga::Statement::Store {
            pointer,
            value: stepped,
        },
        span,
    );
    block.push(
        naga::Statement::If {
            condition: past,
            accept: naga::Block::from_vec(vec![naga::Statement::Break]),
            reject: naga::Block::new(),
        },
        span,
    );
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgx_gpu::wgpu;

    fn validate(module: &naga::Module) -> Result<(), String> {
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(module)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
    }

    fn parse(source: &str) -> naga::Module {
        naga::front::wgsl::parse_str(source).expect("valid WGSL")
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let found = block(instance.request_adapter(&Default::default()))
            .ok()
            .and_then(|adapter| block(adapter.request_device(&Default::default())).ok());
        if found.is_none() && std::env::var_os("ZGX_REQUIRE_GPU").is_some() {
            panic!("ZGX_REQUIRE_GPU, but no device");
        }
        found
    }

    /// `module`'s `main` run over 64 invocations, `input` bound read-only
    /// at 0 and 64 words written at 1: the words, the device waited for (a
    /// test that doesn't end hangs here).
    fn run(
        (device, queue): &(wgpu::Device, wgpu::Queue),
        module: naga::Module,
        input: &[u32],
    ) -> Vec<u32> {
        use wgpu::util::DeviceExt;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(to_wgsl(&module).expect("valid").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(input),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 256);
        queue.submit([encoder.finish()]);
        staging.map_async(wgpu::MapMode::Read, .., |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device");
        bytemuck::pod_collect_to_vec(&staging.slice(..).get_mapped_range())
    }

    const HEAD: &str = "
        @group(0) @binding(0) var<storage, read> input: array<u32>;
        @group(0) @binding(1) var<storage, read_write> output: array<u32>;
    ";

    /// `body` as `main`'s, `n` counting what it ran, written out.
    fn shader(functions: &str, body: &str) -> String {
        format!(
            "{HEAD}
            {functions}
            @compute @workgroup_size(64)
            fn main(@builtin(local_invocation_index) i: u32) {{
                var n = input[0] & 0u;
                {body}
                output[i] = n;
            }}"
        )
    }

    #[test]
    fn every_loop_ends_within_the_cap_an_invocation() {
        let Some(device) = device() else {
            return;
        };
        let cap = 1000;
        let cases = [
            ("loop {}", "", "loop { n += 1u; }"),
            ("nested", "", "loop { loop { n += 1u; } n += 1u; }"),
            (
                "a loop in a called function, called in a loop",
                "fn spin(n: ptr<function, u32>) { loop { *n += 1u; } }",
                "for (var k = 0u; k < 1000000u; k++) { spin(&n); }",
            ),
            (
                "a bound read from a buffer",
                "",
                "for (var k = 0u; k < input[0]; k++) { n += 1u; }",
            ),
            (
                "while and continuing, break if",
                "",
                "var k = 0u; loop { n += 1u; continuing { k += 1u; break if k == 0xffffffffu; } } \
                 while (true) { n += 1u; }",
            ),
        ];
        for (what, functions, body) in cases {
            let source = shader(functions, body);
            let bounded = bound_loops(parse(&source), cap).unwrap();
            validate(&bounded).unwrap_or_else(|e| panic!("{what}: {e}"));
            let out = run(&device, bounded, &[u32::MAX]);
            // Each invocation ran at most the cap's iterations in all.
            for &n in &out[..64] {
                assert!(n <= cap, "{what}: {n} iterations past {cap}");
                assert!(n >= cap - 2, "{what}: stopped early, at {n}");
            }
        }
    }

    #[test]
    fn a_shader_within_the_cap_runs_as_it_did() {
        let Some(device) = device() else {
            return;
        };
        // Loops, nested and called, finishing well within the cap: the same
        // words bounded as not, bit for bit.
        let source = format!(
            "{HEAD}
            fn mix_in(x: u32, k: u32) -> u32 {{
                var y = x;
                for (var j = 0u; j < k % 7u; j++) {{ y = y * 2654435761u + j; }}
                return y;
            }}
            @compute @workgroup_size(64)
            fn main(@builtin(local_invocation_index) i: u32) {{
                var x = i * 40503u + input[0];
                for (var k = 0u; k < 50u; k++) {{
                    var m = 0u;
                    while (m < k % 5u) {{ x ^= x >> 3u; m++; }}
                    x = mix_in(x, k);
                }}
                output[i] = x;
            }}"
        );
        let plain = run(&device, parse(&source), &[12345]);
        let bounded = bound_loops(parse(&source), 1 << 20).unwrap();
        validate(&bounded).unwrap();
        assert_eq!(run(&device, bounded, &[12345]), plain);
    }

    #[test]
    fn a_barrier_in_a_loop_is_refused_not_broken() {
        // One invocation breaking out at the cap while others wait at the
        // barrier is a hang: refused, directly or through a call, and kept
        // outside a loop.
        let barrier_in = |function: &str, body: &str| {
            format!(
                "{HEAD}
                var<workgroup> shared_words: array<u32, 64>;
                {function}
                @compute @workgroup_size(64)
                fn main(@builtin(local_invocation_index) i: u32) {{
                    {body}
                    output[i] = shared_words[i] + input[0];
                }}"
            )
        };
        let direct = barrier_in(
            "",
            "for (var k = 0u; k < 4u; k++) { shared_words[i] = k; workgroupBarrier(); }",
        );
        let called = barrier_in(
            "fn sync() { storageBarrier(); } fn deeper() { sync(); }",
            "loop { deeper(); if input[1] == 0u { break; } }",
        );
        let outside = barrier_in(
            "",
            "for (var k = 0u; k < 4u; k++) { shared_words[i] = k; } workgroupBarrier();",
        );
        for source in [&direct, &called, &outside] {
            assert!(validate(&parse(source)).is_ok());
        }
        assert!(bound_loops(parse(&direct), 1000).is_err());
        assert!(bound_loops(parse(&called), 1000).is_err());
        let kept = bound_loops(parse(&outside), 1000).unwrap();
        assert!(validate(&kept).is_ok());
    }

    #[test]
    fn work_through_the_call_graph_is_weighed_without_running_or_inlining_it() {
        // f0 calls f1 twice, ... f39 calls f40 twice: 2^40 calls an
        // invocation, no loop, a few kilobytes. Weighed once a function, in
        // a moment, and refused.
        let mut source = String::from(HEAD);
        source.push_str("fn f40(x: u32) -> u32 { return x * 3u + 1u; }\n");
        for k in (0..40).rev() {
            source.push_str(&format!(
                "fn f{k}(x: u32) -> u32 {{ return f{n}(x) + f{n}(x + 1u); }}\n",
                n = k + 1
            ));
        }
        source.push_str(
            "@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { output[i] = f0(input[0]); }",
        );
        let module = parse(&source);
        validate(&module).unwrap();
        assert!(
            bound_loops(parse(&source), 1000).is_ok(),
            "no loop to bound"
        );
        let started = std::time::Instant::now();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        let costs = cost(&module, &info).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        assert!(costs[0] > 1 << 40, "{}", costs[0]);
        let refused = sandbox(module, "main", 64, &Limits::default());
        assert!(refused.is_err_and(|why| why.contains("cost")));
    }

    #[test]
    fn a_heavier_loop_body_gets_fewer_iterations_and_too_heavy_a_run_is_refused() {
        let looping = shader("", "loop { n += input[n % 4u] * 3u + 1u; }");
        let light = sandbox(parse(&looping), "main", 64, &Limits::default()).unwrap();
        // A body ten times as heavy, the same invocations: about a tenth.
        let heavy_body = (0..10)
            .map(|_| "n += input[n % 4u] * 3u + 1u;")
            .collect::<String>();
        let heavier = shader("", &format!("loop {{ {heavy_body} }}"));
        let heavy = sandbox(parse(&heavier), "main", 64, &Limits::default()).unwrap();
        assert!(
            heavy.cap * 5 < light.cap,
            "{} against {}",
            heavy.cap,
            light.cap
        );
        for run in [&light, &heavy] {
            assert!(run.work <= Limits::default().work);
        }
        // So many invocations that each would loop fewer than 64 times.
        assert!(sandbox(parse(&looping), "main", 1 << 20, &Limits::default()).is_err());
        // No loop: cost times invocations alone.
        let straight = shader("", "n = input[0] * 3u;");
        assert!(sandbox(parse(&straight), "main", 1 << 16, &Limits::default()).is_ok());
        assert!(sandbox(parse(&straight), "main", 1 << 30, &Limits::default()).is_err());
    }

    #[test]
    fn memory_and_size_past_the_limits_are_refused() {
        let private = format!(
            "{HEAD} var<private> big: array<f32, 8192>; \
            @compute @workgroup_size(1) fn main() {{ big[0] = f32(input[0]); output[0] = u32(big[0]); }}"
        );
        assert!(
            sandbox(parse(&private), "main", 1, &Limits::default())
                .is_err_and(|w| w.contains("memory"))
        );
        let local = shader(
            "",
            "var big: array<u32, 8192>; big[input[0] % 8192u] = 1u; n = big[0];",
        );
        assert!(
            sandbox(parse(&local), "main", 1, &Limits::default())
                .is_err_and(|w| w.contains("memory"))
        );
        let shared = format!(
            "{HEAD} var<workgroup> big: array<u32, 8192>; \
            @compute @workgroup_size(1) fn main() {{ big[0] = input[0]; output[0] = big[0]; }}"
        );
        assert!(
            sandbox(parse(&shared), "main", 1, &Limits::default())
                .is_err_and(|w| w.contains("workgroup"))
        );
        // Many nodes: a long straight run of statements, past a limit of 2,000.
        let long = shader("", &"n = n * 3u + input[0];".repeat(1000));
        let few = Limits {
            nodes: 2000,
            ..Limits::default()
        };
        assert!(sandbox(parse(&long), "main", 1, &few).is_err_and(|w| w.contains("nodes")));
    }

    #[test]
    fn a_whole_value_weighs_its_size_and_one_past_the_memory_is_refused() {
        // A million-element array copied whole in a loop: 16 MB a copy, one
        // value past an invocation's memory. Refused.
        let storage = "
            @group(0) @binding(0) var<storage, read> a: array<vec4<f32>, 1000000>;
            @group(0) @binding(1) var<storage, read_write> b: array<vec4<f32>, 1000000>;
            @compute @workgroup_size(1) fn main() {
                for (var k = 0u; k < 1000000u; k++) { b = a; }
            }";
        let refused = sandbox(parse(storage), "main", 1, &Limits::default());
        assert!(
            refused.is_err_and(|why| why.contains("a value of")),
            "copying arrays whole"
        );
        // An 8,000-byte private array copied there and back (16,000 bytes
        // held, within the memory), each copy weighing 500: its loop gets
        // far fewer iterations than a light one.
        let private = format!(
            "{HEAD}
            var<private> big: array<vec4<u32>, 500>;
            @compute @workgroup_size(1) fn main() {{
                var p: array<vec4<u32>, 500>;
                loop {{ p = big; p[0].x += input[0]; big = p; }}
                output[0] = big[0].x;
            }}"
        );
        let heavy = sandbox(parse(&private), "main", 1, &Limits::default());
        let light = sandbox(
            parse(&shader("", "loop { n += input[0]; }")),
            "main",
            1,
            &Limits::default(),
        )
        .unwrap();
        let heavy = heavy.unwrap();
        // Two loads and two stores of 500 units each an iteration: at least
        // 2,000, against some 25 for the light loop.
        assert!(heavy.work / (u64::from(heavy.cap) + 1) >= 2000);
        assert!(
            heavy.cap * 50 < light.cap,
            "{} against {}",
            heavy.cap,
            light.cap
        );
        assert!(heavy.work <= Limits::default().work);
    }

    #[test]
    fn values_passed_and_returned_weigh_their_size() {
        // A function taking and returning a 4 KB array by value, called in
        // a loop: each call moves it in and out.
        let source = format!(
            "{HEAD}
            fn relay(x: array<vec4<u32>, 256>) -> array<vec4<u32>, 256> {{ return x; }}
            @compute @workgroup_size(1) fn main() {{
                var a: array<vec4<u32>, 256>;
                for (var k = 0u; k < input[0]; k++) {{ a = relay(a); }}
                output[0] = a[0].x;
            }}"
        );
        let module = parse(&source);
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        // Argument, result, the store, the return: four moves of 256 units.
        assert!(cost(&module, &info).unwrap()[0] >= 4 * 256);
    }
}
