//! Numeric affine mapping: endpoints may run script; source changes do not.
use crate::{Vm, node, value::Value};
use mlua::{Function, Lua, Table, UserData};
use std::rc::Rc;
use zgx::{compose::NodeHandle, reactive::Signal};

#[derive(Clone)]
enum Endpoint {
    Number(f64),
    State(Signal<Value>),
    Function(Function),
}
impl Endpoint {
    fn parse(value: mlua::Value) -> mlua::Result<Self> {
        if let Some(state) = node::state_of(&value) {
            return Ok(Self::State(state));
        }
        match value {
            mlua::Value::Integer(n) => Ok(Self::Number(n as f64)),
            mlua::Value::Number(n) => Ok(Self::Number(n)),
            mlua::Value::Function(f) => Ok(Self::Function(f)),
            _ => Err(mlua::Error::runtime(
                "ui.map endpoints take numbers, numeric states, or functions",
            )),
        }
    }
    fn read(&self, vm: &Vm) -> mlua::Result<f64> {
        match self {
            Self::Number(n) => Ok(*n),
            Self::State(state) => state
                .with(|v| v.as_f64())
                .ok_or_else(|| mlua::Error::runtime("ui.map endpoint state must contain a number")),
            Self::Function(f) => vm.call(f, ()),
        }
    }
}
fn pair(table: Table) -> mlua::Result<[Endpoint; 2]> {
    if table.raw_len() != 2 {
        return Err(mlua::Error::runtime(
            "ui.map from and to each take two endpoints",
        ));
    }
    Ok([
        Endpoint::parse(table.raw_get(1)?)?,
        Endpoint::parse(table.raw_get(2)?)?,
    ])
}
fn affine(source: f64, [from, end, to, target]: [f64; 4], clamp: bool) -> f32 {
    let finite = |n: f64| {
        if n.is_finite() {
            n.clamp(-(f32::MAX as f64), f32::MAX as f64) as f32
        } else {
            0.
        }
    };
    if ![source, from, end, to, target]
        .iter()
        .all(|v| v.is_finite())
        || from == end
    {
        return finite(to);
    }
    let mut t = (source - from) / (end - from);
    if clamp {
        t = t.clamp(0., 1.);
    }
    let value = to + (target - to) * t;
    if value.is_finite() {
        finite(value)
    } else {
        finite(to)
    }
}
fn set_endpoint(params: &Signal<[f64; 4]>, index: usize, number: f64) {
    params.mutate(|values| {
        if values[index] == number {
            return false;
        }
        values[index] = number;
        true
    });
}

pub(crate) struct MappedUd(pub Signal<Value>);
impl UserData for MappedUd {}

pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    ui.set(
        "map",
        lua.create_function(|lua, (source, options): (mlua::Value, Table)| {
            let source = node::state_of(&source).ok_or_else(|| {
                mlua::Error::runtime("ui.map takes a numeric state as its source")
            })?;
            let from = pair(options.get("from")?)?;
            let to = pair(options.get("to")?)?;
            let clamp = options.get::<Option<bool>>("clamp")?.unwrap_or(false);
            let env = node::env(lua)?;
            let params = env.runtime.signal([0., 0., 0., 0.]);
            let value = env.runtime.signal(Value::Num(0.));
            for (index, endpoint) in from.into_iter().chain(to).enumerate() {
                // A literal has no dependencies. Each other endpoint owns
                // its effect: changing one must not run unrelated functions.
                if let Endpoint::Number(number) = endpoint {
                    set_endpoint(&params, index, number);
                    continue;
                }
                let (vm, endpoints) = (env.vm.clone(), params.clone());
                let update_endpoint = env.runtime.effect_now(move || {
                    let Some(vm) = vm.upgrade() else {
                        return;
                    };
                    match endpoint.read(&vm) {
                        Ok(number) => set_endpoint(&endpoints, index, number),
                        Err(error) => vm.report(error),
                    }
                });
                node::retain(&env, Box::new(update_endpoint));
            }
            let output = value.clone();
            let update_value = env.runtime.effect_now(move || {
                let source = source.with(|v| v.as_f64().unwrap_or(f64::NAN));
                output.set(Value::Num(affine(source, params.get(), clamp) as f64));
            });
            node::retain(&env, Box::new(update_value));
            Ok(MappedUd(value))
        })?,
    )
}

/// Keep the explicit mapping's installation apart from ordinary style
/// lowering, including its closure identities and scalar representation.
#[cold]
#[inline(never)]
pub(crate) fn install_binding(
    view: zgx::compose::View,
    node: &node::Node,
    prop: node::Prop,
    value: Signal<Value>,
    vm: Rc<Vm>,
) -> zgx::compose::View {
    // Competing inset styles retain the ordinary cascade. Its Lua
    // functions are kept apart, so offsets still resolve natively.
    let direct = node.classes.is_empty()
        && node.variant_interaction.is_none()
        && !node
            .pseudo
            .iter()
            .any(|(_, styles)| styles.iter().any(|(p, _)| *p == prop));
    if direct {
        view.on_mount(binding(value, prop == node::Prop::Top, vm))
    } else {
        view
    }
}

/// Subscribe before the ordinary style effect is installed. The explicit
/// inset writes first; the style cascade then either keeps that same style
/// (no invalidation) or applies its overriding inset through the normal path.
fn binding(value: Signal<Value>, top: bool, vm: Rc<Vm>) -> impl FnOnce(NodeHandle) {
    let mounted = Rc::new(std::cell::OnceCell::<NodeHandle>::new());
    let target = mounted.clone();
    let binding = vm.runtime.effect_now(move || {
        let value = value.with(|v| v.as_f64().unwrap_or(0.) as f32);
        let Some(handle) = target.get() else {
            return;
        };
        let Some(scene) = handle.scene() else {
            return;
        };
        let mut scene = scene.borrow_mut();
        if scene.contains(handle.node()) {
            scene.set_mapped_position(handle.node(), (!top).then_some(value), top.then_some(value));
        }
    });
    move |handle| {
        let _ = mounted.set(handle.clone());
        handle.retain(binding);
    }
}

#[cfg(test)]
mod tests {
    use super::affine;
    #[test]
    fn affine_handles_reversed_degenerate_and_nonfinite_ranges() {
        assert_eq!(affine(50., [100., 0., 4., 104.], true), 54.);
        assert_eq!(affine(200., [100., 0., 4., 104.], true), 4.);
        assert_eq!(affine(99., [0., 0., 4., 104.], true), 4.);
        assert_eq!(affine(f64::NAN, [0., 100., 4., 104.], true), 4.);
        assert!(affine(1., [0., 1., f64::INFINITY, f64::MAX], false).is_finite());
    }
}
