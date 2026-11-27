//! Script limits for a bench: an app's (`Limits::app_budgets`), or with
//! `ZGX_GATE` set (CI's instruction gate runs the benches under cachegrind,
//! some fifty times slower), time budgets a thousand times as long, so a
//! simulated run is measured, not stopped. Only the benches read it; apps
//! never do.
pub fn limits() -> zgx_script::Limits {
    let limits = zgx_script::Limits::app_budgets();
    if zgx::env::var_os("ZGX_GATE").is_none() {
        return limits;
    }
    zgx_script::Limits {
        load: limits.load.map(|d| d * 1000),
        call: limits.call.map(|d| d * 1000),
        ..limits
    }
}
