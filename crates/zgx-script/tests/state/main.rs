//! State, calls, I/O, plugins and the host API. One test binary for the area (see AGENTS.md):
//! a test file goes in the group for its area, listed here.
// Each file loads its own `common` (a Harness of its own, which files
// add methods to), as it did as a binary of its own.
#![allow(clippy::duplicate_mod)]
#[path = "../app_api.rs"]
mod app_api;
#[path = "../async_handlers.rs"]
mod async_handlers;
#[path = "../calls.rs"]
mod calls;
#[path = "../form_values.rs"]
mod form_values;
#[path = "../io.rs"]
mod io;
#[path = "../large_data.rs"]
mod large_data;
#[path = "../module_overrides.rs"]
mod module_overrides;
#[path = "../notify.rs"]
mod notify;
#[path = "../ownership.rs"]
mod ownership;
#[path = "../packages.rs"]
mod packages;
#[path = "../paged.rs"]
mod paged;
#[path = "../plugin_branches.rs"]
mod plugin_branches;
#[path = "../plugin_context.rs"]
mod plugin_context;
#[path = "../plugin_elements.rs"]
mod plugin_elements;
#[path = "../plugin_filters.rs"]
mod plugin_filters;
#[path = "../plugin_hooks.rs"]
mod plugin_hooks;
#[path = "../plugin_packages.rs"]
mod plugin_packages;
#[path = "../plugin_report.rs"]
mod plugin_report;
#[path = "../plugin_rows.rs"]
mod plugin_rows;
#[path = "../plugin_shared.rs"]
mod plugin_shared;
#[path = "../plugin_targets.rs"]
mod plugin_targets;
#[path = "../plugins.rs"]
mod plugins;
#[path = "../reload_cache.rs"]
mod reload_cache;
#[path = "../router.rs"]
mod router;
#[path = "../script.rs"]
mod script;
#[path = "../socket.rs"]
mod socket;
#[path = "../state.rs"]
mod state;
#[path = "../streamed_append.rs"]
mod streamed_append;
#[path = "../streaming.rs"]
mod streaming;
#[path = "../ui_errors.rs"]
mod ui_errors;
#[path = "../upload.rs"]
mod upload;
#[path = "../windows.rs"]
mod windows;
