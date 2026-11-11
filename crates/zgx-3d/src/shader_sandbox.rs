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
}
