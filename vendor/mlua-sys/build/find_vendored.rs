#![allow(dead_code)]

pub fn probe_lua() {
    #[cfg(feature = "lua55")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua55);

    #[cfg(feature = "lua54")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua54);

    #[cfg(feature = "lua53")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua53);

    #[cfg(feature = "lua52")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua52);

    #[cfg(feature = "lua51")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51);

    #[cfg(feature = "luajit")]
    let artifacts = luajit_src::Build::new()
        .lua52compat(cfg!(feature = "luajit52"))
        .build();

    // zgx patch: Luau's asserts (`LUAU_ASSERT`) are live unless NDEBUG is
    // defined, and luau0-src defines it only through `LUAU_CXXFLAGS`, which
    // an app built from crates.io never sets: scripts ran with every assert.
    // Defined here, in the process that runs luau0-src's builder, unless the
    // crate is built with debug assertions; a `LUAU_CXXFLAGS` of the caller's
    // is kept beside it. `ndebug` tells the crates depending on this one.
    #[cfg(feature = "luau")]
    {
        let flags = std::env::var("LUAU_CXXFLAGS").unwrap_or_default();
        let defined = flags.split_whitespace().any(|f| {
            let f = f.trim_start_matches(['-', '/']);
            f == "DNDEBUG" || f.starts_with("DNDEBUG=")
        });
        if std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_none() && !defined {
            // SAFETY: a build script's one thread, before anything reads it.
            unsafe { std::env::set_var("LUAU_CXXFLAGS", format!("{flags} -DNDEBUG")) };
        }
        if defined || std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_none() {
            println!("cargo:ndebug=1");
        }
    }
    #[cfg(feature = "luau")]
    let artifacts = luau0_src::Build::new()
        .enable_codegen(cfg!(feature = "luau-codegen"))
        .set_max_cstack_size(1000000)
        .set_vector_size(if cfg!(feature = "luau-vector4") { 4 } else { 3 })
        .build();

    artifacts.print_cargo_metadata();
}
